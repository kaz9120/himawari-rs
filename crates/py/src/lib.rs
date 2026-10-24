use numpy::{IntoPyArray, PyArray1};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use rayon::prelude::*;

use himawari_core::packed_sfen::{PSV_BYTES, PackedSfenValue, unpack};

use himawari_engine::nnue::{
    ARCH, CONCAT, EFF_ACT, EFF_GENERIC, EFF_IN, EFF_MAX_ACTIVE, EFF_OUT, EFF_PAIR, EFFECT_LEN,
    FT_IN, FT_OUT, L1_OUT, L1_PAD, L2_OUT, L2_PAD, L3_OUT, LAST_HIDDEN, MOVE_FROM_CLASSES,
    MOVE_NONE, MOVE_TO_CLASSES, NnueNetwork, SUB_OUT, effect_active, effect_counts, effect_labels,
    evaluate_scalar, halfkp_active, move_labels,
};
use himawari_engine::posgen::{chunk_specs, generate_chunk};
use himawari_engine::{nnue_io, submodel};

const SIGMOID_SCALE: f32 = 600.0;
/// 評価値スケール（ADR-0036）。量子化を戻すのに使う
const FV_SCALE: i32 = 16;

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// 1レコードの抽出結果。指し手ラベルは補助ヘッド用（ADR-0129）。
/// 利きラベルは事前学習用で、要求されたときだけ入る（ADR-0133）。
struct Sample {
    stm: Vec<u32>,
    opp: Vec<u32>,
    target: f32,
    move_from: i64,
    move_to: i64,
    effect: Option<([u8; EFFECT_LEN], [u8; EFFECT_LEN])>,
    /// 側の入力特徴。手番視点・非手番視点の順に `EFF_ACTIVE` 個ずつ。F2は81個で、
    /// 表し方を替えた構成（ADR-0245）は上限の個数まで詰め物（`EFF_IN`）で埋める。
    eff_inputs: Option<(Vec<u32>, Vec<u32>)>,
}

/// 1レコードを特徴・教師信号・指し手ラベルに変換する。
/// 展開に失敗した局面と、score_limitで除外した局面はNoneを返す。
///
/// `score_clamp` が正なら、評価値をその絶対値へ丸めてから教師信号にする。
/// 教師データの9.8%は詰みスコア（±29000以上）で、`sigmoid(score/600)` は
/// |score| >= 4144 で飽和する。丸めずに通すと教師信号が0か1に張り付き、
/// モデルは有限の出力で届かない値を目指し続ける（ADR-0126）。
///
/// `effect` が真のときだけ利きラベルを計算する（ADR-0133）。利きを使わない
/// 学習に計算の費用を払わせない。
fn extract_one(
    raw: &[u8; PSV_BYTES],
    lambda_: f32,
    score_limit: i16,
    score_clamp: i16,
    effect: bool,
    eff_inputs: bool,
) -> Option<Sample> {
    let rec = PackedSfenValue::from_bytes(raw);
    if score_limit > 0 && rec.score.abs() >= score_limit {
        return None;
    }
    let pos = unpack(&rec.sfen, rec.game_ply).ok()?;

    let stm = pos.side_to_move();
    let mut stm_feats = Vec::new();
    let mut opp_feats = Vec::new();
    halfkp_active(&pos, stm, &mut stm_feats);
    halfkp_active(&pos, stm.flip(), &mut opp_feats);

    let score = if score_clamp > 0 {
        rec.score.clamp(-score_clamp, score_clamp)
    } else {
        rec.score
    };
    let p_score = sigmoid(f32::from(score) / SIGMOID_SCALE);
    let p_result = (f32::from(rec.game_result) + 1.0) / 2.0;
    let target = lambda_ * p_score + (1.0 - lambda_) * p_result;
    let (move_from, move_to) = move_labels(rec.move16, stm);
    // posが生きているあいだに計算する。展開し直すと局面を2度作ることになる
    let effect = effect.then(|| effect_labels(&pos));
    let eff_inputs = eff_inputs.then(|| side_inputs(&pos));

    Some(Sample {
        stm: stm_feats,
        opp: opp_feats,
        target,
        move_from,
        move_to,
        effect,
        eff_inputs,
    })
}

/// 1局面1視点あたりの側の入力特徴の数。F2は升ごとに1つで81、表し方を替えた構成は上限。
const EFF_ACTIVE: usize = if EFF_GENERIC { EFF_MAX_ACTIVE } else { 81 };

/// 側の入力特徴を手番視点・非手番視点の順に返す（ADR-0237・0245）。推論と同じ
/// `effect_active`・`side_feat` を通すので、学習と推論で特徴の作り方がずれない。
fn effect_inputs(pos: &himawari_core::Position) -> (Vec<u32>, Vec<u32>) {
    let stm = pos.side_to_move();
    if EFF_GENERIC {
        return (
            himawari_engine::side_feat::active_padded(pos, stm),
            himawari_engine::side_feat::active_padded(pos, stm.flip()),
        );
    }
    let counts = effect_counts(pos);
    let mut a = [0u32; 81];
    let mut b = [0u32; 81];
    effect_active(pos, stm, &counts, &mut a);
    effect_active(pos, stm.flip(), &counts, &mut b);
    (a.to_vec(), b.to_vec())
}

/// 側の入力特徴の後ろへ、その視点の玉の窓（ADR-0246）を `SUB_LEN` 個つなぐ。
/// 学習側は1行を `EFF_ACTIVE` 個と `SUB_LEN` 個に割って使う。持たない構成では何もしない。
fn side_inputs(pos: &himawari_core::Position) -> (Vec<u32>, Vec<u32>) {
    let (mut a, mut b) = effect_inputs(pos);
    if SUB_OUT != 0 {
        let stm = pos.side_to_move();
        let mut w = [0u16; submodel::SUB_LEN];
        for (v, c) in [(&mut a, stm), (&mut b, stm.flip())] {
            submodel::inputs(pos, c, &mut w);
            v.extend(w.iter().map(|&x| u32::from(x)));
        }
    }
    (a, b)
}

#[pyfunction]
#[pyo3(signature = (record, lambda_ = 0.7, score_limit = 0, score_clamp = 0))]
fn extract_features(
    record: &[u8],
    lambda_: f32,
    score_limit: i16,
    score_clamp: i16,
) -> PyResult<Option<(Vec<u32>, Vec<u32>, f32)>> {
    if record.len() != PSV_BYTES {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "レコードは{PSV_BYTES}バイト必要（{}バイト渡された）",
            record.len()
        )));
    }
    let raw: &[u8; PSV_BYTES] = record.try_into().unwrap();
    Ok(
        extract_one(raw, lambda_, score_limit, score_clamp, false, false)
            .map(|s| (s.stm, s.opp, s.target)),
    )
}

/// バッチ分のレコードをまとめてEmbeddingBag形式へ変換する（ADR-0065）。
///
/// 戻り値は `(stm_idx, stm_off, opp_idx, opp_off, targets, move_from, move_to,
/// eff_short, eff_long, eff_stm, eff_opp)` の11本。`move_from`・`move_to` は補助ヘッド用の指し手
/// ラベルで、取れなかった局面は-1になる（ADR-0129）。末尾2本は利き数の
/// ラベルで、1局面あたり `EFFECT_LEN` 個ずつ並ぶ（ADR-0133）。`effect` が
/// 偽なら長さ0で返る。最後の2本は利きの入力特徴（ADR-0237）で、1局面あたり
/// `EFF_ACTIVE` 個ずつ並ぶ（F2は81）。`effect_inputs` が偽なら長さ0で返る。
/// 抽出はrayonで並列に行い、その間GILを解放する。
type BatchArrays<'py> = (
    Bound<'py, PyArray1<i64>>,
    Bound<'py, PyArray1<i64>>,
    Bound<'py, PyArray1<i64>>,
    Bound<'py, PyArray1<i64>>,
    Bound<'py, PyArray1<f32>>,
    Bound<'py, PyArray1<i64>>,
    Bound<'py, PyArray1<i64>>,
    Bound<'py, PyArray1<u8>>,
    Bound<'py, PyArray1<u8>>,
    Bound<'py, PyArray1<i64>>,
    Bound<'py, PyArray1<i64>>,
);

#[pyfunction]
#[pyo3(signature = (records, lambda_ = 0.7, score_limit = 0, score_clamp = 0, effect = false, strict = false, effect_inputs = false))]
// 引数はPython側の呼び出し規約そのもの（上のsignature）
#[allow(clippy::too_many_arguments)]
fn extract_batch<'py>(
    py: Python<'py>,
    records: &[u8],
    lambda_: f32,
    score_limit: i16,
    score_clamp: i16,
    effect: bool,
    strict: bool,
    effect_inputs: bool,
) -> PyResult<BatchArrays<'py>> {
    if records.len() % PSV_BYTES != 0 {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "バイト数が{PSV_BYTES}の倍数でない: {}",
            records.len()
        )));
    }

    let (
        dropped,
        stm_idx,
        stm_off,
        opp_idx,
        opp_off,
        targets,
        mv_from,
        mv_to,
        eff_short,
        eff_long,
        eff_stm,
        eff_opp,
    ) = py.allow_threads(|| {
        let per: Vec<Option<Sample>> = records
            .par_chunks_exact(PSV_BYTES)
            .map(|chunk| {
                let raw: &[u8; PSV_BYTES] = chunk.try_into().unwrap();
                extract_one(
                    raw,
                    lambda_,
                    score_limit,
                    score_clamp,
                    effect,
                    effect_inputs,
                )
            })
            .collect();

        let n = per.len();
        let dropped = per.iter().filter(|s| s.is_none()).count();
        let mut stm_idx: Vec<i64> = Vec::with_capacity(n * 40);
        let mut opp_idx: Vec<i64> = Vec::with_capacity(n * 40);
        let mut stm_off: Vec<i64> = Vec::with_capacity(n);
        let mut opp_off: Vec<i64> = Vec::with_capacity(n);
        let mut targets: Vec<f32> = Vec::with_capacity(n);
        let mut mv_from: Vec<i64> = Vec::with_capacity(n);
        let mut mv_to: Vec<i64> = Vec::with_capacity(n);
        let eff_cap = if effect { n * EFFECT_LEN } else { 0 };
        let mut eff_short: Vec<u8> = Vec::with_capacity(eff_cap);
        let mut eff_long: Vec<u8> = Vec::with_capacity(eff_cap);
        let in_cap = if effect_inputs { n * EFF_ACTIVE } else { 0 };
        let mut eff_stm: Vec<i64> = Vec::with_capacity(in_cap);
        let mut eff_opp: Vec<i64> = Vec::with_capacity(in_cap);

        for s in per.into_iter().flatten() {
            stm_off.push(stm_idx.len() as i64);
            opp_off.push(opp_idx.len() as i64);
            stm_idx.extend(s.stm.iter().map(|&x| i64::from(x)));
            opp_idx.extend(s.opp.iter().map(|&x| i64::from(x)));
            targets.push(s.target);
            mv_from.push(s.move_from);
            mv_to.push(s.move_to);
            if let Some((short, long)) = s.effect {
                eff_short.extend_from_slice(&short);
                eff_long.extend_from_slice(&long);
            }
            if let Some((a, b)) = s.eff_inputs {
                eff_stm.extend(a.iter().map(|&x| i64::from(x)));
                eff_opp.extend(b.iter().map(|&x| i64::from(x)));
            }
        }
        (
            dropped, stm_idx, stm_off, opp_idx, opp_off, targets, mv_from, mv_to, eff_short,
            eff_long, eff_stm, eff_opp,
        )
    });

    // レコードの並びが意味を持つ用途（ADR-0185のランキング群）では、
    // 黙って落とすと整列が壊れる。strictでは落とした時点で失敗させる
    if strict && dropped > 0 {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "strict指定で{dropped}レコードを抽出できなかった（復元失敗かscore_limit）"
        )));
    }

    Ok((
        stm_idx.into_pyarray(py),
        stm_off.into_pyarray(py),
        opp_idx.into_pyarray(py),
        opp_off.into_pyarray(py),
        targets.into_pyarray(py),
        mv_from.into_pyarray(py),
        mv_to.into_pyarray(py),
        eff_short.into_pyarray(py),
        eff_long.into_pyarray(py),
        eff_stm.into_pyarray(py),
        eff_opp.into_pyarray(py),
    ))
}

/// 局面をその場で作り、`extract_batch` と同じ11本の配列で返す（ADR-0133）。
/// 利きの入力特徴（末尾2本）は持たず、長さ0で返る。
///
/// 利き予測は決定的な構造タスクなので、的は局面の分布に依存しない。教師
/// データファイルが要らず、**生成した局面を使い捨てにすれば同じ局面は
/// 二度と出ない。** 訓練損失がそのまま未見データの損失になる。
///
/// 利きラベルは常に計算する。この関数は利き予測のためにあり、切る意味がない。
/// 評価値の的はないので `targets` は0.5で埋め、指し手ラベルは無効値にする。
/// **`--lambda-value 0` で使う前提である。**
///
/// 並列化の単位は `GEN_CHUNK` 局面の塊で、塊ごとの種は `seed` から決定的に
/// 導く。結果を塊の順で連結するので、同じ `seed` からは並列度によらず
/// 同じ局面列が出る。
#[pyfunction]
#[pyo3(signature = (count, seed, max_plies = 256))]
fn generate_batch<'py>(
    py: Python<'py>,
    count: usize,
    seed: u64,
    max_plies: u16,
) -> PyResult<BatchArrays<'py>> {
    if max_plies == 0 {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "max_pliesは1以上が要る（0では局面を作れない）",
        ));
    }

    let (stm_idx, stm_off, opp_idx, opp_off, targets, mv_from, mv_to, eff_short, eff_long) = py
        .allow_threads(|| {
            let chunks: Vec<_> = chunk_specs(count, seed)
                .into_par_iter()
                .map(|(s, c)| generate_chunk(s, c, max_plies))
                .collect();

            let mut stm_idx: Vec<i64> = Vec::with_capacity(count * 38);
            let mut opp_idx: Vec<i64> = Vec::with_capacity(count * 38);
            let mut stm_off: Vec<i64> = Vec::with_capacity(count);
            let mut opp_off: Vec<i64> = Vec::with_capacity(count);
            let mut eff_short: Vec<u8> = Vec::with_capacity(count * EFFECT_LEN);
            let mut eff_long: Vec<u8> = Vec::with_capacity(count * EFFECT_LEN);

            for s in chunks.into_iter().flatten() {
                stm_off.push(stm_idx.len() as i64);
                opp_off.push(opp_idx.len() as i64);
                stm_idx.extend(s.stm.iter().map(|&x| i64::from(x)));
                opp_idx.extend(s.opp.iter().map(|&x| i64::from(x)));
                eff_short.extend_from_slice(&s.short);
                eff_long.extend_from_slice(&s.long);
            }
            // 評価値の的はない。0.5は勝率50%で、λ_value=0なら勾配へ効かない
            let targets = vec![0.5f32; count];
            let mv_from = vec![MOVE_NONE; count];
            let mv_to = vec![MOVE_NONE; count];
            (
                stm_idx, stm_off, opp_idx, opp_off, targets, mv_from, mv_to, eff_short, eff_long,
            )
        });

    Ok((
        stm_idx.into_pyarray(py),
        stm_off.into_pyarray(py),
        opp_idx.into_pyarray(py),
        opp_off.into_pyarray(py),
        targets.into_pyarray(py),
        mv_from.into_pyarray(py),
        mv_to.into_pyarray(py),
        eff_short.into_pyarray(py),
        eff_long.into_pyarray(py),
        Vec::<i64>::new().into_pyarray(py),
        Vec::<i64>::new().into_pyarray(py),
    ))
}

/// 学習側が渡す重みの長さを検査する。次元の取り違えは、書けてしまうと
/// 読み込み時のバイト数不一致まで気づけない（ADR-0127）。
fn check_len(name: &str, got: usize, want: usize) -> PyResult<()> {
    if got != want {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "{name}の要素数が合わない: {got}（このビルドは{ARCH}なので{want}）"
        )));
    }
    Ok(())
}

#[pyfunction]
#[pyo3(signature = (path, lineage, ft_w, ft_b, w2, b2, w3, b3, w_out, b_out, w4=None, b4=None, eff_w=None, eff_b=None, sub_w=None))]
// 引数の並びがそのままPython側の呼び出し規約になる（上のsignature）。層ごとの
// 重みとバイアスを構造体へまとめるには、受け渡す型をpyclassにしてquantize.pyの
// 呼び出しも書き換えることになるので、平らな引数で受ける
#[allow(clippy::too_many_arguments)]
fn save_hmwr(
    path: &str,
    lineage: &str,
    ft_w: Vec<i16>,
    ft_b: Vec<i16>,
    w2: Vec<i8>,
    b2: Vec<i32>,
    w3: Vec<i8>,
    b3: Vec<i32>,
    w_out: Vec<i8>,
    b_out: i32,
    // 4層構成でだけ渡す隠れ層3
    w4: Option<Vec<i8>>,
    b4: Option<Vec<i32>>,
    // 利きのaccumulatorを持つ構成でだけ渡す（ADR-0237）
    eff_w: Option<Vec<i16>>,
    eff_b: Option<Vec<i16>>,
    // 玉の近傍の部分モデルを持つ構成でだけ渡す（ADR-0246）。学習側の形のf32
    sub_w: Option<Vec<f32>>,
) -> PyResult<()> {
    check_len("ft_w", ft_w.len(), FT_IN * FT_OUT)?;
    // 学習側は常にi16で渡す。i8ビルドでは範囲検査を通してから格納する
    // （ADR-0138）。飽和する重みは黙って切り詰めずエラーにする
    let ft_w = himawari_engine::nnue::ft_w_from_i16(ft_w)
        .map_err(pyo3::exceptions::PyValueError::new_err)?;
    check_len("ft_b", ft_b.len(), FT_OUT)?;
    check_len("w2", w2.len(), L1_OUT * CONCAT)?;
    check_len("b2", b2.len(), L1_OUT)?;
    check_len("w3", w3.len(), L2_OUT * L1_OUT)?;
    check_len("b3", b3.len(), L2_OUT)?;
    check_len("w_out", w_out.len(), LAST_HIDDEN)?;
    let (w4, b4) = match (w4, b4) {
        (Some(w), Some(b)) => {
            check_len("w4", w.len(), L3_OUT * L2_OUT)?;
            check_len("b4", b.len(), L3_OUT)?;
            (himawari_engine::nnue::pad_rows(&w, L2_OUT, L2_PAD), b)
        }
        (None, None) => {
            check_len("w4", 0, L3_OUT * L2_OUT)?;
            (Vec::new(), Vec::new())
        }
        _ => {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "w4とb4は両方渡すか、両方省く",
            ));
        }
    };
    let eff_w = eff_w.unwrap_or_default();
    let eff_b = eff_b.unwrap_or_default();
    check_len("eff_w", eff_w.len(), EFF_IN * EFF_OUT)?;
    check_len("eff_b", eff_b.len(), EFF_OUT)?;
    let sub_w = sub_w.unwrap_or_default();
    check_len("sub_w", sub_w.len(), submodel::SUB_PARAMS)?;
    let net = NnueNetwork {
        ft_w,
        ft_b,
        w2,
        w2_sparse: Vec::new(),
        b2,
        // 学習側はパディングを持たない。推論の幅へ広げる
        w3: himawari_engine::nnue::pad_rows(&w3, L1_OUT, L1_PAD),
        b3,
        w4,
        b4,
        w_out,
        b_out,
        eff_w,
        eff_b,
        sub: submodel::SubNet::from_raw(sub_w),
    }
    .finish();
    let mut f = std::fs::File::create(path)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    nnue_io::save(&net, lineage, &mut f)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    Ok(())
}

/// `.hmwr` を読み、学習側が使うf32の重みへ戻す（ADR-0130）。
///
/// 量子化の逆変換なので、元のf32とは丸めのぶんだけ違う。凍結して使う
/// ぶんには差が動かないので影響しない。構成が違うファイルはいまの
/// ビルド構成へ合わせて読む（`makenet --resize` と同じ扱い）。
///
/// 戻り値の `src_arch` は元ファイルの構成で、後段の層を初期値に使ってよいかを
/// 学習側が判断するために返す。
#[pyfunction]
fn load_hmwr<'py>(py: Python<'py>, path: &str) -> PyResult<Bound<'py, PyDict>> {
    let mut f = std::fs::File::open(path)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    let (net, lineage, src) = nnue_io::load_resized_with_dims(&mut f)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e))?;

    let out_w_scale = SIGMOID_SCALE * FV_SCALE as f32 / 127.0;
    let f32v =
        |v: &[i8], scale: f32| -> Vec<f32> { v.iter().map(|&x| f32::from(x) / scale).collect() };

    let d = PyDict::new(py);
    d.set_item(
        "ft_w",
        net.ft_w
            .iter()
            .map(|&x| f32::from(x) / 127.0)
            .collect::<Vec<f32>>()
            .into_pyarray(py),
    )?;
    d.set_item(
        "ft_b",
        net.ft_b
            .iter()
            .map(|&x| f32::from(x) / 127.0)
            .collect::<Vec<f32>>()
            .into_pyarray(py),
    )?;
    d.set_item("w2", f32v(&net.w2, 64.0).into_pyarray(py))?;
    d.set_item("w3", unpad(&net.w3, L1_OUT, L1_PAD, 64.0).into_pyarray(py))?;
    d.set_item("w4", unpad(&net.w4, L2_OUT, L2_PAD, 64.0).into_pyarray(py))?;
    d.set_item("w_out", f32v(&net.w_out, out_w_scale).into_pyarray(py))?;
    for (key, v) in [("b2", &net.b2), ("b3", &net.b3), ("b4", &net.b4)] {
        let scaled: Vec<f32> = v.iter().map(|&x| x as f32 / (64.0 * 127.0)).collect();
        d.set_item(key, scaled.into_pyarray(py))?;
    }
    d.set_item(
        "b_out",
        net.b_out as f32 / (SIGMOID_SCALE * FV_SCALE as f32),
    )?;
    for (key, v) in [("eff_w", &net.eff_w), ("eff_b", &net.eff_b)] {
        let scaled: Vec<f32> = v.iter().map(|&x| f32::from(x) / 127.0).collect();
        d.set_item(key, scaled.into_pyarray(py))?;
    }
    d.set_item("sub_w", net.sub.raw.clone().into_pyarray(py))?;
    d.set_item("lineage", lineage)?;
    d.set_item(
        "src_arch",
        format!("{}x{}x{}x{}", src[1], src[2], src[3], src[4]),
    )?;
    Ok(d)
}

/// `.hmwr` を読み、psvのレコードを推論側の全計算で評価する（ADR-0237）。
///
/// 学習と推論の一致テストに使う。値は手番視点・歩=90スケールの整数で、
/// 探索が見るのと同じ `evaluate_scalar` を通す。復元できないレコードは
/// エラーにする。黙って落とすと学習側の出力との整列が壊れる。
#[pyfunction]
fn evaluate_records(path: &str, records: &[u8]) -> PyResult<Vec<i32>> {
    if records.len() % PSV_BYTES != 0 {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "バイト数が{PSV_BYTES}の倍数でない: {}",
            records.len()
        )));
    }
    let mut f = std::fs::File::open(path)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    let (net, _) =
        nnue_io::load(&mut f).map_err(PyErr::new::<pyo3::exceptions::PyValueError, _>)?;
    records
        .chunks_exact(PSV_BYTES)
        .map(|chunk| {
            let raw: &[u8; PSV_BYTES] = chunk.try_into().unwrap();
            let rec = PackedSfenValue::from_bytes(raw);
            let pos = unpack(&rec.sfen, rec.game_ply).map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("復元できない: {e:?}"))
            })?;
            Ok(evaluate_scalar(&net, &pos))
        })
        .collect()
}

/// ゼロ埋め列を落としてf32へ戻す。学習側はパディングを持たない。
fn unpad(rows: &[i8], used: usize, stride: usize, scale: f32) -> Vec<f32> {
    rows.chunks_exact(stride.max(1))
        .flat_map(|row| row[..used].iter().map(|&x| f32::from(x) / scale))
        .collect()
}

#[pymodule]
fn himawari(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(extract_features, m)?)?;
    m.add_function(wrap_pyfunction!(extract_batch, m)?)?;
    m.add_function(wrap_pyfunction!(generate_batch, m)?)?;
    m.add_function(wrap_pyfunction!(save_hmwr, m)?)?;
    m.add_function(wrap_pyfunction!(load_hmwr, m)?)?;
    m.add_function(wrap_pyfunction!(evaluate_records, m)?)?;
    m.add("FT_IN", FT_IN)?;
    m.add("FT_OUT", FT_OUT)?;
    m.add("CONCAT", CONCAT)?;
    m.add("L1_OUT", L1_OUT)?;
    m.add("L2_OUT", L2_OUT)?;
    m.add("L3_OUT", L3_OUT)?;
    m.add("ARCH", ARCH)?;
    m.add("MOVE_FROM_CLASSES", MOVE_FROM_CLASSES)?;
    m.add("MOVE_TO_CLASSES", MOVE_TO_CLASSES)?;
    m.add("EFFECT_LEN", EFFECT_LEN)?;
    m.add("EFF_IN", EFF_IN)?;
    m.add("EFF_OUT", EFF_OUT)?;
    // 表し方を替えた構成（ADR-0245）。学習側は詰め物の行 `EFF_IN` を足して持つ
    m.add("EFF_GENERIC", EFF_GENERIC)?;
    m.add("EFF_PAIR", EFF_PAIR)?;
    m.add("EFF_ACT", EFF_ACT)?;
    m.add("EFF_ACTIVE", EFF_ACTIVE)?;
    // 玉の近傍の部分モデル（ADR-0246）。SUB_OUTが0なら持たない
    m.add("SUB_OUT", SUB_OUT)?;
    m.add("SUB_LEN", submodel::SUB_LEN)?;
    m.add("SUB_SLOT", submodel::SUB_SLOT)?;
    m.add("SUB_WIN", submodel::WIN)?;
    m.add("SUB_TOKENS", submodel::TOKENS)?;
    m.add("SUB_EMB", submodel::EMB)?;
    m.add("SUB_EXTRA", submodel::EXTRA)?;
    m.add("SUB_HID", submodel::HID)?;
    m.add(
        "SUB_HAND_SCALE",
        himawari_core::PieceType::HAND_KINDS
            .iter()
            .map(|&pt| submodel::hand_scale(pt))
            .collect::<Vec<f32>>(),
    )?;
    // 側の表のうちHalfKPの区画の起点（ADR-0245の習甦方式）。無ければ-1
    m.add(
        "EFF_KP_BASE",
        himawari_engine::side_feat::KP_BASE.map_or(-1, |b| b as i64),
    )?;
    Ok(())
}

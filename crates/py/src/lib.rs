use numpy::{IntoPyArray, PyArray1};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use rayon::prelude::*;

use himawari_core::packed_sfen::{PSV_BYTES, PackedSfenValue, unpack};

use himawari_engine::nnue::{
    ARCH, CONCAT, EFFECT_LEN, FT_IN, FT_OUT, L1_OUT, L1_PAD, L2_OUT, L2_PAD, L3_OUT, LAST_HIDDEN,
    MOVE_FROM_CLASSES, MOVE_NONE, MOVE_TO_CLASSES, NnueNetwork, effect_labels, halfkp_active,
    move_labels,
};
use himawari_engine::nnue_io;
use himawari_engine::posgen::{chunk_specs, generate_chunk};

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

    Some(Sample {
        stm: stm_feats,
        opp: opp_feats,
        target,
        move_from,
        move_to,
        effect,
    })
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
        extract_one(raw, lambda_, score_limit, score_clamp, false)
            .map(|s| (s.stm, s.opp, s.target)),
    )
}

/// バッチ分のレコードをまとめてEmbeddingBag形式へ変換する（ADR-0065）。
///
/// 戻り値は `(stm_idx, stm_off, opp_idx, opp_off, targets, move_from, move_to,
/// eff_short, eff_long)` の9本。`move_from`・`move_to` は補助ヘッド用の指し手
/// ラベルで、取れなかった局面は-1になる（ADR-0129）。末尾2本は利き数の
/// ラベルで、1局面あたり `EFFECT_LEN` 個ずつ並ぶ（ADR-0133）。`effect` が
/// 偽なら長さ0で返る。抽出はrayonで並列に行い、その間GILを解放する。
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
);

#[pyfunction]
#[pyo3(signature = (records, lambda_ = 0.7, score_limit = 0, score_clamp = 0, effect = false))]
fn extract_batch<'py>(
    py: Python<'py>,
    records: &[u8],
    lambda_: f32,
    score_limit: i16,
    score_clamp: i16,
    effect: bool,
) -> PyResult<BatchArrays<'py>> {
    if records.len() % PSV_BYTES != 0 {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "バイト数が{PSV_BYTES}の倍数でない: {}",
            records.len()
        )));
    }

    let (stm_idx, stm_off, opp_idx, opp_off, targets, mv_from, mv_to, eff_short, eff_long) = py
        .allow_threads(|| {
            let per: Vec<Option<Sample>> = records
                .par_chunks_exact(PSV_BYTES)
                .map(|chunk| {
                    let raw: &[u8; PSV_BYTES] = chunk.try_into().unwrap();
                    extract_one(raw, lambda_, score_limit, score_clamp, effect)
                })
                .collect();

            let n = per.len();
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
            }
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
    ))
}

/// 局面をその場で作り、`extract_batch` と同じ9本の配列で返す（ADR-0133）。
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
#[pyo3(signature = (path, lineage, ft_w, ft_b, w2, b2, w3, b3, w_out, b_out, w4=None, b4=None))]
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
    d.set_item("lineage", lineage)?;
    d.set_item(
        "src_arch",
        format!("{}x{}x{}x{}", src[1], src[2], src[3], src[4]),
    )?;
    Ok(d)
}

/// `.hmwr` のFTだけを、元ファイルの次元のまま読む（ADR-0132）。
///
/// `load_hmwr` はいまのビルド構成へ合わせて読むので、FT256の拡張から
/// FT768のネットを読むとFTが256へ切り詰められる。表現蒸留は太いFTの
/// 出力そのものを教師にするため、切り詰めない口が要る。
///
/// 戻り値の `ft_out` が元ファイルのFT幅で、`ft_w` はその幅で並ぶ。
/// 後段は読まない。教師に使うのはFTだけである。
#[pyfunction]
fn load_hmwr_ft<'py>(py: Python<'py>, path: &str) -> PyResult<Bound<'py, PyDict>> {
    let mut f = std::fs::File::open(path)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    let (ft_w, ft_b, src) =
        nnue_io::load_ft(&mut f).map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e))?;

    let dequant = |v: &[i16]| -> Vec<f32> {
        v.iter()
            .map(|&x| f32::from(x) / 127.0)
            .collect::<Vec<f32>>()
    };

    let d = PyDict::new(py);
    d.set_item("ft_w", dequant(&ft_w).into_pyarray(py))?;
    d.set_item("ft_b", dequant(&ft_b).into_pyarray(py))?;
    d.set_item("ft_out", src[1])?;
    d.set_item(
        "src_arch",
        format!("{}x{}x{}x{}", src[1], src[2], src[3], src[4]),
    )?;
    Ok(d)
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
    m.add_function(wrap_pyfunction!(load_hmwr_ft, m)?)?;
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
    Ok(())
}

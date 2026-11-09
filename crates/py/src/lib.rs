use numpy::{IntoPyArray, PyArray1};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use rayon::prelude::*;

use himawari_core::packed_sfen::{PSV_BYTES, PackedSfenValue, unpack};

use himawari_engine::nnue::{
    ARCH, CONCAT, EFF_GENERIC, EFF_IN, EFF_MAX_ACTIVE, EFF_OUT, FT_IN, FT_OUT, L1_OUT, L1_PAD,
    L2_OUT, LAST_HIDDEN, NnueNetwork, effect_active, effect_counts, evaluate_scalar, halfkp_active,
    move_labels,
};
use himawari_engine::nnue_io;

const SIGMOID_SCALE: f32 = 600.0;
/// 評価値スケール。量子化を戻すのに使う
const FV_SCALE: i32 = 16;

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// 1レコードの抽出結果。指し手ラベルは補助ヘッド用。
struct Sample {
    stm: Vec<u32>,
    opp: Vec<u32>,
    target: f32,
    move_from: i64,
    move_to: i64,
    /// 側の入力特徴。手番視点・非手番視点の順に `EFF_ACTIVE` 個ずつ。F2は81個で、
    /// F2と最小の攻め駒の組は上限の個数まで詰め物（`EFF_IN`）で埋める。
    eff_inputs: Option<(Vec<u32>, Vec<u32>)>,
}

/// 1レコードを特徴・教師信号・指し手ラベルに変換する。
/// 展開に失敗した局面と、score_limitで除外した局面はNoneを返す。
///
/// `score_clamp` が正なら、評価値をその絶対値へ丸めてから教師信号にする。
/// 教師データの9.8%は詰みスコア（±29000以上）で、`sigmoid(score/600)` は
/// |score| >= 4144 で飽和する。丸めずに通すと教師信号が0か1に張り付き、
/// モデルは有限の出力で届かない値を目指し続ける。
fn extract_one(
    raw: &[u8; PSV_BYTES],
    lambda_: f32,
    score_limit: i16,
    score_clamp: i16,
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
    let eff_inputs = eff_inputs.then(|| effect_inputs(&pos));

    Some(Sample {
        stm: stm_feats,
        opp: opp_feats,
        target,
        move_from,
        move_to,
        eff_inputs,
    })
}

/// 1局面1視点あたりの側の入力特徴の数。F2は升ごとに1つで81、F2と最小の攻め駒の組は上限。
const EFF_ACTIVE: usize = if EFF_GENERIC { EFF_MAX_ACTIVE } else { 81 };

/// 側の入力特徴を手番視点・非手番視点の順に返す。推論と同じ
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

/// バッチ分のレコードをまとめてEmbeddingBag形式へ変換する。
///
/// 戻り値は `(stm_idx, stm_off, opp_idx, opp_off, targets, move_from, move_to,
/// eff_stm, eff_opp)` の9本。`move_from`・`move_to` は補助ヘッド用の指し手
/// ラベルで、取れなかった局面は-1になる。最後の2本は利きの入力特徴
/// で、1局面あたり `EFF_ACTIVE` 個ずつ並ぶ（F2は81）。
/// `effect_inputs` が偽なら長さ0で返る。
/// 抽出はrayonで並列に行い、その間GILを解放する。
type BatchArrays<'py> = (
    Bound<'py, PyArray1<i64>>,
    Bound<'py, PyArray1<i64>>,
    Bound<'py, PyArray1<i64>>,
    Bound<'py, PyArray1<i64>>,
    Bound<'py, PyArray1<f32>>,
    Bound<'py, PyArray1<i64>>,
    Bound<'py, PyArray1<i64>>,
    Bound<'py, PyArray1<i64>>,
    Bound<'py, PyArray1<i64>>,
);

#[pyfunction]
#[pyo3(signature = (records, lambda_ = 0.7, score_limit = 0, score_clamp = 0, strict = false, effect_inputs = false))]
fn extract_batch<'py>(
    py: Python<'py>,
    records: &[u8],
    lambda_: f32,
    score_limit: i16,
    score_clamp: i16,
    strict: bool,
    effect_inputs: bool,
) -> PyResult<BatchArrays<'py>> {
    if records.len() % PSV_BYTES != 0 {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "バイト数が{PSV_BYTES}の倍数でない: {}",
            records.len()
        )));
    }

    let (dropped, stm_idx, stm_off, opp_idx, opp_off, targets, mv_from, mv_to, eff_stm, eff_opp) =
        py.allow_threads(|| {
            let per: Vec<Option<Sample>> = records
                .par_chunks_exact(PSV_BYTES)
                .map(|chunk| {
                    let raw: &[u8; PSV_BYTES] = chunk.try_into().unwrap();
                    extract_one(raw, lambda_, score_limit, score_clamp, effect_inputs)
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
                if let Some((a, b)) = s.eff_inputs {
                    eff_stm.extend(a.iter().map(|&x| i64::from(x)));
                    eff_opp.extend(b.iter().map(|&x| i64::from(x)));
                }
            }
            (
                dropped, stm_idx, stm_off, opp_idx, opp_off, targets, mv_from, mv_to, eff_stm,
                eff_opp,
            )
        });

    // レコードの並びが意味を持つ用途（兄弟局面のランキング群）では、
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
        eff_stm.into_pyarray(py),
        eff_opp.into_pyarray(py),
    ))
}

/// 学習側が渡す重みの長さを検査する。次元の取り違えは、書けてしまうと
/// 読み込み時のバイト数不一致まで気づけない。
fn check_len(name: &str, got: usize, want: usize) -> PyResult<()> {
    if got != want {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "{name}の要素数が合わない: {got}（このビルドは{ARCH}なので{want}）"
        )));
    }
    Ok(())
}

#[pyfunction]
#[pyo3(signature = (path, lineage, ft_w, ft_b, w2, b2, w3, b3, w_out, b_out, eff_w=None, eff_b=None))]
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
    // 利きのaccumulatorを持つ構成でだけ渡す
    eff_w: Option<Vec<i16>>,
    eff_b: Option<Vec<i16>>,
) -> PyResult<()> {
    check_len("ft_w", ft_w.len(), FT_IN * FT_OUT)?;
    // 学習側は常にi16で渡す。i8ビルドでは範囲検査を通してから格納する。
    // 飽和する重みは黙って切り詰めずエラーにする
    let ft_w = himawari_engine::nnue::ft_w_from_i16(ft_w)
        .map_err(pyo3::exceptions::PyValueError::new_err)?;
    check_len("ft_b", ft_b.len(), FT_OUT)?;
    check_len("w2", w2.len(), L1_OUT * CONCAT)?;
    check_len("b2", b2.len(), L1_OUT)?;
    check_len("w3", w3.len(), L2_OUT * L1_OUT)?;
    check_len("b3", b3.len(), L2_OUT)?;
    check_len("w_out", w_out.len(), LAST_HIDDEN)?;
    let eff_w = eff_w.unwrap_or_default();
    let eff_b = eff_b.unwrap_or_default();
    check_len("eff_w", eff_w.len(), EFF_IN * EFF_OUT)?;
    check_len("eff_b", eff_b.len(), EFF_OUT)?;
    let net = NnueNetwork {
        ft_w,
        ft_b,
        w2,
        w2_sparse: Vec::new(),
        b2,
        // 学習側はパディングを持たない。推論の幅へ広げる
        w3: himawari_engine::nnue::pad_rows(&w3, L1_OUT, L1_PAD),
        b3,
        w_out,
        b_out,
        eff_w,
        eff_b,
    }
    .finish();
    let mut f = std::fs::File::create(path)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    nnue_io::save(&net, lineage, &mut f)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    Ok(())
}

/// `.hmwr` を読み、学習側が使うf32の重みへ戻す。
///
/// 量子化の逆変換なので、元のf32とは丸めのぶんだけ違う。凍結して使う
/// ぶんには差が動かないので影響しない。構成が違うファイルはいまの
/// ビルド構成へ合わせて読む（`nnue_io::load_resized_with_dims`）。
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
    d.set_item("w_out", f32v(&net.w_out, out_w_scale).into_pyarray(py))?;
    for (key, v) in [("b2", &net.b2), ("b3", &net.b3)] {
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
    d.set_item("lineage", lineage)?;
    d.set_item("src_arch", format!("{}x{}x{}", src[1], src[2], src[3]))?;
    Ok(d)
}

/// `.hmwr` を読み、psvのレコードを推論側の全計算で評価する。
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
    m.add_function(wrap_pyfunction!(save_hmwr, m)?)?;
    m.add_function(wrap_pyfunction!(load_hmwr, m)?)?;
    m.add_function(wrap_pyfunction!(evaluate_records, m)?)?;
    m.add("FT_IN", FT_IN)?;
    m.add("FT_OUT", FT_OUT)?;
    m.add("CONCAT", CONCAT)?;
    m.add("L1_OUT", L1_OUT)?;
    m.add("L2_OUT", L2_OUT)?;
    m.add("ARCH", ARCH)?;
    m.add("EFF_IN", EFF_IN)?;
    m.add("EFF_OUT", EFF_OUT)?;
    // F2と最小の攻め駒の組。学習側は詰め物の行 `EFF_IN` を足して持つ
    m.add("EFF_GENERIC", EFF_GENERIC)?;
    m.add("EFF_ACTIVE", EFF_ACTIVE)?;
    Ok(())
}

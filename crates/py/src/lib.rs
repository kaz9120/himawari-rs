use numpy::{IntoPyArray, PyArray1};
use pyo3::prelude::*;
use rayon::prelude::*;

use himawari_core::packed_sfen::{PSV_BYTES, PackedSfenValue, unpack};
use himawari_engine::nnue::{FT_IN, FT_OUT, HIDDEN, NnueNetwork, halfkp_active};
use himawari_engine::nnue_io;

const SIGMOID_SCALE: f32 = 600.0;

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// 1レコードを (手番側特徴, 相手側特徴, 教師信号) に変換する。
/// 展開に失敗した局面と、score_limitで除外した局面はNoneを返す。
///
/// `score_clamp` が正なら、評価値をその絶対値へ丸めてから教師信号にする。
/// 教師データの9.8%は詰みスコア（±29000以上）で、`sigmoid(score/600)` は
/// |score| >= 4144 で飽和する。丸めずに通すと教師信号が0か1に張り付き、
/// モデルは有限の出力で届かない値を目指し続ける（ADR-0126）。
fn extract_one(
    raw: &[u8; PSV_BYTES],
    lambda_: f32,
    score_limit: i16,
    score_clamp: i16,
) -> Option<(Vec<u32>, Vec<u32>, f32)> {
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

    Some((stm_feats, opp_feats, target))
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
    Ok(extract_one(raw, lambda_, score_limit, score_clamp))
}

/// バッチ分のレコードをまとめてEmbeddingBag形式へ変換する（ADR-0065）。
///
/// 戻り値は `(stm_idx, stm_off, opp_idx, opp_off, targets)` の5本。
/// 抽出はrayonで並列に行い、その間GILを解放する。
type BatchArrays<'py> = (
    Bound<'py, PyArray1<i64>>,
    Bound<'py, PyArray1<i64>>,
    Bound<'py, PyArray1<i64>>,
    Bound<'py, PyArray1<i64>>,
    Bound<'py, PyArray1<f32>>,
);

#[pyfunction]
#[pyo3(signature = (records, lambda_ = 0.7, score_limit = 0, score_clamp = 0))]
fn extract_batch<'py>(
    py: Python<'py>,
    records: &[u8],
    lambda_: f32,
    score_limit: i16,
    score_clamp: i16,
) -> PyResult<BatchArrays<'py>> {
    if records.len() % PSV_BYTES != 0 {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "バイト数が{PSV_BYTES}の倍数でない: {}",
            records.len()
        )));
    }

    let (stm_idx, stm_off, opp_idx, opp_off, targets) = py.allow_threads(|| {
        let per: Vec<Option<(Vec<u32>, Vec<u32>, f32)>> = records
            .par_chunks_exact(PSV_BYTES)
            .map(|chunk| {
                let raw: &[u8; PSV_BYTES] = chunk.try_into().unwrap();
                extract_one(raw, lambda_, score_limit, score_clamp)
            })
            .collect();

        let n = per.len();
        let mut stm_idx: Vec<i64> = Vec::with_capacity(n * 40);
        let mut opp_idx: Vec<i64> = Vec::with_capacity(n * 40);
        let mut stm_off: Vec<i64> = Vec::with_capacity(n);
        let mut opp_off: Vec<i64> = Vec::with_capacity(n);
        let mut targets: Vec<f32> = Vec::with_capacity(n);

        for (stm, opp, t) in per.into_iter().flatten() {
            stm_off.push(stm_idx.len() as i64);
            opp_off.push(opp_idx.len() as i64);
            stm_idx.extend(stm.iter().map(|&x| i64::from(x)));
            opp_idx.extend(opp.iter().map(|&x| i64::from(x)));
            targets.push(t);
        }
        (stm_idx, stm_off, opp_idx, opp_off, targets)
    });

    Ok((
        stm_idx.into_pyarray(py),
        stm_off.into_pyarray(py),
        opp_idx.into_pyarray(py),
        opp_off.into_pyarray(py),
        targets.into_pyarray(py),
    ))
}

#[pyfunction]
#[pyo3(signature = (path, lineage, ft_w, ft_b, w2, b2, w3, b3, w4, b4))]
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
    w4: Vec<i8>,
    b4: i32,
) -> PyResult<()> {
    let net = NnueNetwork {
        ft_w,
        ft_b,
        w2,
        b2,
        w3,
        b3,
        w4,
        b4,
    };
    let mut f = std::fs::File::create(path)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    nnue_io::save(&net, lineage, &mut f)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    Ok(())
}

#[pymodule]
fn himawari(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(extract_features, m)?)?;
    m.add_function(wrap_pyfunction!(extract_batch, m)?)?;
    m.add_function(wrap_pyfunction!(save_hmwr, m)?)?;
    m.add("FT_IN", FT_IN)?;
    m.add("FT_OUT", FT_OUT)?;
    m.add("HIDDEN", HIDDEN)?;
    Ok(())
}

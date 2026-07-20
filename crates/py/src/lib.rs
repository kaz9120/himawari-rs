use pyo3::prelude::*;

use himawari_core::packed_sfen::{PSV_BYTES, PackedSfenValue, unpack};
use himawari_engine::nnue::{FT_IN, FT_OUT, HIDDEN, NnueNetwork, halfkp_active};
use himawari_engine::nnue_io;

const SIGMOID_SCALE: f32 = 600.0;

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

#[pyfunction]
#[pyo3(signature = (record, lambda_ = 0.7, score_limit = 0))]
fn extract_features(
    record: &[u8],
    lambda_: f32,
    score_limit: i16,
) -> PyResult<Option<(Vec<u32>, Vec<u32>, f32)>> {
    if record.len() != PSV_BYTES {
        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "レコードは{PSV_BYTES}バイト必要（{}バイト渡された）",
            record.len()
        )));
    }
    let raw: &[u8; PSV_BYTES] = record.try_into().unwrap();
    let rec = PackedSfenValue::from_bytes(raw);

    if score_limit > 0 && rec.score.abs() >= score_limit {
        return Ok(None);
    }
    let pos = match unpack(&rec.sfen, rec.game_ply) {
        Ok(p) => p,
        Err(_) => return Ok(None),
    };

    let stm = pos.side_to_move();
    let mut stm_feats = Vec::new();
    let mut opp_feats = Vec::new();
    halfkp_active(&pos, stm, &mut stm_feats);
    halfkp_active(&pos, stm.flip(), &mut opp_feats);

    let p_score = sigmoid(f32::from(rec.score) / SIGMOID_SCALE);
    let p_result = (f32::from(rec.game_result) + 1.0) / 2.0;
    let target = lambda_ * p_score + (1.0 - lambda_) * p_result;

    Ok(Some((stm_feats, opp_feats, target)))
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

#[pyfunction]
fn load_hmwr(py: Python<'_>, path: &str) -> PyResult<PyObject> {
    let mut f = std::fs::File::open(path)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    let (net, lineage) =
        nnue_io::load(&mut f).map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e))?;

    let dict = pyo3::types::PyDict::new(py);
    dict.set_item("ft_w", net.ft_w)?;
    dict.set_item("ft_b", net.ft_b)?;
    dict.set_item("w2", net.w2)?;
    dict.set_item("b2", net.b2)?;
    dict.set_item("w3", net.w3)?;
    dict.set_item("b3", net.b3)?;
    dict.set_item("w4", net.w4)?;
    dict.set_item("b4", net.b4)?;
    dict.set_item("lineage", lineage)?;
    Ok(dict.into())
}

#[pymodule]
fn himawari(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(extract_features, m)?)?;
    m.add_function(wrap_pyfunction!(save_hmwr, m)?)?;
    m.add_function(wrap_pyfunction!(load_hmwr, m)?)?;
    m.add("FT_IN", FT_IN)?;
    m.add("FT_OUT", FT_OUT)?;
    m.add("HIDDEN", HIDDEN)?;
    Ok(())
}

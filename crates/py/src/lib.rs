use pyo3::prelude::*;

use himawari_core::Position;
use himawari_core::packed_sfen::{PSV_BYTES, PackedSfenValue, unpack};
use himawari_core::piece::PieceType;
use himawari_core::types::{Color, Square};
use himawari_engine::nnue::{
    EFFECT_IN, EFFECT_OUT, FT_IN, FT_OUT, HIDDEN, NnueNetwork, effect_active, halfkp_active,
};
use himawari_engine::nnue_io;

const SIGMOID_SCALE: f32 = 600.0;
const N_PIECE_ENC: u16 = 113;
const KINGLINE_IN: usize = 2 * 8 * N_PIECE_ENC as usize;

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

#[pyfunction]
#[pyo3(signature = (record, lambda_ = 0.7, score_limit = 0))]
fn extract_features(
    record: &[u8],
    lambda_: f32,
    score_limit: i16,
) -> PyResult<Option<(Vec<u32>, Vec<u32>, Vec<u16>, f32)>> {
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
    let mut ef_feats = Vec::new();
    effect_active(&pos, &mut ef_feats);

    let p_score = sigmoid(f32::from(rec.score) / SIGMOID_SCALE);
    let p_result = (f32::from(rec.game_result) + 1.0) / 2.0;
    let target = lambda_ * p_score + (1.0 - lambda_) * p_result;

    Ok(Some((stm_feats, opp_feats, ef_feats, target)))
}

/// 8方向の (df, dr) 定数。手番視点で正規化済みの方向。
const DIRECTIONS: [(i32, i32); 8] = [
    (0, -1),  // N (forward)
    (1, -1),  // NE
    (1, 0),   // E
    (1, 1),   // SE
    (0, 1),   // S (backward)
    (-1, 1),  // SW
    (-1, 0),  // W
    (-1, -1), // NW
];

fn pt_to_idx(pt: PieceType) -> u16 {
    match pt {
        PieceType::PAWN => 0,
        PieceType::LANCE => 1,
        PieceType::KNIGHT => 2,
        PieceType::SILVER => 3,
        PieceType::GOLD => 4,
        PieceType::BISHOP => 5,
        PieceType::ROOK => 6,
        PieceType::KING => 7,
        PieceType::PRO_PAWN => 8,
        PieceType::PRO_LANCE => 9,
        PieceType::PRO_KNIGHT => 10,
        PieceType::PRO_SILVER => 11,
        PieceType::HORSE => 12,
        PieceType::DRAGON => 13,
        _ => 0,
    }
}

fn piece_enc(is_friendly: bool, pt: PieceType, distance: u8) -> u16 {
    let color_idx: u16 = if is_friendly { 0 } else { 1 };
    let pt_idx: u16 = pt_to_idx(pt);
    let dist_bucket: u16 = match distance {
        1 => 0,
        2 => 1,
        3 | 4 => 2,
        _ => 3,
    };
    color_idx * 14 * 4 + pt_idx * 4 + dist_bucket
}

const EMPTY_ENC: u16 = 2 * 14 * 4; // = 112

fn kingline_active(pos: &Position, out: &mut Vec<u16>) {
    out.clear();
    let stm = pos.side_to_move();
    let kings = [pos.king(stm), pos.king(stm.flip())];

    for (slot, &k) in kings.iter().enumerate() {
        let k_norm = if stm == Color::Black { k } else { k.inv() };
        let kf = k_norm.file().0 as i32;
        let kr = k_norm.rank().0 as i32;
        let base = slot as u16 * 8 * N_PIECE_ENC;

        for (dir_idx, &(df, dr)) in DIRECTIONS.iter().enumerate() {
            let dir_base = base + dir_idx as u16 * N_PIECE_ENC;
            // 手番視点の方向を実座標に変換
            let (rdf, rdr) = if stm == Color::Black {
                (df, dr)
            } else {
                (-df, -dr)
            };
            let mut f = k.file().0 as i32 + rdf;
            let mut r = k.rank().0 as i32 + rdr;
            let mut found = false;
            let mut dist: u8 = 0;
            while (0..9).contains(&f) && (0..9).contains(&r) {
                dist += 1;
                let sq = Square::from_index((f * 9 + r) as u8);
                let pc = pos.piece_on(sq);
                if !pc.is_empty() {
                    let is_friendly = pc.color() == stm;
                    let enc = piece_enc(is_friendly, pc.piece_type(), dist);
                    out.push(dir_base + enc);
                    found = true;
                    break;
                }
                f += rdf;
                r += rdr;
            }
            if !found {
                out.push(dir_base + EMPTY_ENC);
            }
        }
    }
}

#[pyfunction]
#[pyo3(signature = (record, lambda_ = 0.7, score_limit = 0, arch = "halfkp_effect"))]
fn extract_features_v2(
    record: &[u8],
    lambda_: f32,
    score_limit: i16,
    arch: &str,
) -> PyResult<Option<(Vec<u32>, Vec<u32>, Vec<u16>, f32)>> {
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

    let mut ef_feats = Vec::new();
    match arch {
        "halfkp_effect" => effect_active(&pos, &mut ef_feats),
        "halfkp_kingline" => kingline_active(&pos, &mut ef_feats),
        "halfkp" => {} // no second tower
        _ => {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "未知のarch: {arch}"
            )));
        }
    }

    let p_score = sigmoid(f32::from(rec.score) / SIGMOID_SCALE);
    let p_result = (f32::from(rec.game_result) + 1.0) / 2.0;
    let target = lambda_ * p_score + (1.0 - lambda_) * p_result;

    Ok(Some((stm_feats, opp_feats, ef_feats, target)))
}

#[pyfunction]
#[pyo3(signature = (path, lineage, ft_w, ft_b, ef_w, ef_b, w2, b2, w3, b3, w4, b4))]
#[allow(clippy::too_many_arguments)]
fn save_hmwr(
    path: &str,
    lineage: &str,
    ft_w: Vec<i16>,
    ft_b: Vec<i16>,
    ef_w: Vec<i16>,
    ef_b: Vec<i16>,
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
        ef_w,
        ef_b,
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
    dict.set_item("ef_w", net.ef_w)?;
    dict.set_item("ef_b", net.ef_b)?;
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
    m.add_function(wrap_pyfunction!(extract_features_v2, m)?)?;
    m.add_function(wrap_pyfunction!(save_hmwr, m)?)?;
    m.add_function(wrap_pyfunction!(load_hmwr, m)?)?;
    m.add("FT_IN", FT_IN)?;
    m.add("FT_OUT", FT_OUT)?;
    m.add("EFFECT_IN", EFFECT_IN)?;
    m.add("EFFECT_OUT", EFFECT_OUT)?;
    m.add("HIDDEN", HIDDEN)?;
    m.add("KINGLINE_IN", KINGLINE_IN)?;
    Ok(())
}

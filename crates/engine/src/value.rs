//! 評価値の符号化（ADR-0024, 0026）。

pub type Value = i32;

pub const VALUE_ZERO: Value = 0;
pub const VALUE_DRAW: Value = 0;
pub const VALUE_MATE: Value = 32000;
pub const VALUE_INFINITE: Value = 32601;
pub const VALUE_NONE: Value = 32602;
/// 優等局面の値（ADR-0026）。ply補正しない。
pub const VALUE_SUPERIOR: Value = 28000;

pub const MAX_PLY: usize = 128;
pub const VALUE_MATE_IN_MAX_PLY: Value = VALUE_MATE - MAX_PLY as Value;
pub const VALUE_MATED_IN_MAX_PLY: Value = -VALUE_MATE_IN_MAX_PLY;

#[inline]
pub const fn mate_in(ply: usize) -> Value {
    VALUE_MATE - ply as Value
}

#[inline]
pub const fn mated_in(ply: usize) -> Value {
    -VALUE_MATE + ply as Value
}

/// 置換表へ保存する値（詰みスコアから根までの距離を除く。ADR-0024）。
#[inline]
pub fn value_to_tt(v: Value, ply: usize) -> i16 {
    debug_assert!(v.abs() < VALUE_NONE);
    let adj = if v >= VALUE_MATE_IN_MAX_PLY {
        v + ply as Value
    } else if v <= VALUE_MATED_IN_MAX_PLY {
        v - ply as Value
    } else {
        v
    };
    adj as i16
}

/// 置換表から取り出した値（現在plyを加えて詰みスコアを復元）。
#[inline]
pub fn value_from_tt(v: i16, ply: usize) -> Value {
    let v = Value::from(v);
    if v >= VALUE_MATE_IN_MAX_PLY {
        v - ply as Value
    } else if v <= VALUE_MATED_IN_MAX_PLY {
        v + ply as Value
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mate_roundtrip_via_tt() {
        // ply 10で発見したmate_in(13)を保存し、ply 6で取り出す
        let v = mate_in(13);
        let stored = value_to_tt(v, 10);
        let restored = value_from_tt(stored, 6);
        // 保存側で根基準になり、取得側で現plyが加わる
        assert_eq!(restored, VALUE_MATE - 13 + 10 - 6);
        // 通常値は不変
        assert_eq!(value_from_tt(value_to_tt(123, 42), 7), 123);
        assert_eq!(value_from_tt(value_to_tt(-456, 3), 99), -456);
    }
}

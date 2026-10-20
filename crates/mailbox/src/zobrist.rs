//! Zobristのキーと、局面に付いて回る値の表（ADR-0241の第4回）。
//!
//! 探索へ接続したとき置換表と千日手の判定をcoreと共有できるよう、乱数の生成も
//! キーの定義もcore（ADR-0015・0046・0085・0109）と同じにする。乱数は同じ種の
//! splitmix64で作るので、値はcoreの `zobrist` と1ビットも違わない（照合のテストで確かめる）。
//! 升の添字は81升の通し番号（coreのSquareと同じ）。

/// 手番のXOR値。board_keyのbit 0が手番を表す。
pub const SIDE: u64 = 1;

/// 置換表キーの合成に使う拡散定数（coreと同じ）。
pub const HAND_MIX: u64 = 0x9E37_79B9_7F4A_7C15;

const fn splitmix64(state: u64) -> (u64, u64) {
    let s = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = s;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (s, z ^ (z >> 31))
}

/// PSQ[駒の符号][81升の通し番号]。
pub static PSQ: [[u64; 81]; 32] = {
    let mut t = [[0u64; 81]; 32];
    let mut state = 20260717u64;
    let mut p = 2;
    while p < 32 {
        if p % 16 >= 2 {
            let mut sq = 0;
            while sq < 81 {
                let (s, v) = splitmix64(state);
                state = s;
                t[p][sq] = v & !1;
                sq += 1;
            }
        }
        p += 1;
    }
    t
};

/// 歩構造キーの持ち歩の成分。[色][枚数 0..=18]。
pub static HAND_PAWN: [[u64; 19]; 2] = {
    let mut t = [[0u64; 19]; 2];
    let mut state = 20260721u64;
    let mut c = 0;
    while c < 2 {
        let mut n = 0;
        while n < 19 {
            let (s, v) = splitmix64(state);
            state = s;
            t[c][n] = v;
            n += 1;
        }
        c += 1;
    }
    t
};

/// coreの `Hand`（u32に7駒種を詰めた値）での駒種ごとのシフト量。hand_keyに使う。
pub const HAND_SHIFT: [u32; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 18, 0, 6, 10, 14, 22, 25];

/// 手駒の枚数の欄の幅のマスク（`HAND_SHIFT` の位置から）。歩は5ビット、香桂銀金は3ビット、角飛は2ビット。
pub const HAND_MASK: [u32; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 7, 31, 7, 7, 7, 3, 3];

/// 駒種の材料価値（coreの `PIECE_VALUE` と同じ）。
pub const PIECE_VALUE: [i32; 16] = [
    0, 0, 540, 540, 540, 540, 945, 1395, 15000, 540, 90, 315, 405, 495, 855, 990,
];

/// 小駒（香・桂・銀・金とその成駒）の駒種マスク（coreと同じ）。
const MINOR_PIECE_MASK: u16 =
    (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5) | (1 << 9) | (1 << 11) | (1 << 12) | (1 << 13);

#[inline]
pub const fn is_minor_piece(pt: u8) -> bool {
    MINOR_PIECE_MASK & (1 << pt) != 0
}

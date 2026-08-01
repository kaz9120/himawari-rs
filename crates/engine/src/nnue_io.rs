//! NNUE評価ファイルの独自フォーマット（ADR-0037）。
//!
//! ヘッダ: マジック・フォーマット版・アーキテクチャ記述子
//! （各塔の次元）・学習来歴文字列・重みハッシュ。以降は
//! リトルエンディアンの重み列。アーキテクチャ不一致・ハッシュ
//! 不一致は読み込みエラーにする（気づかず壊れたネットで
//! 対局する事故を防ぐ）。
//!
//! 版3で隠れ層の幅を2つ（L1・L2）持つようにした（ADR-0127）。
//! 版2は幅を1つしか書かないため、L1とL2が同じ構成でだけ読める。

use std::io::{Read, Write};

use crate::nnue::{
    ARCH, CONCAT, FT_IN, FT_OUT, L1_OUT, L1_PAD, L2_OUT, NnueNetwork, pad_l2_weights,
};

const MAGIC: &[u8; 8] = b"HMWRNNUE";
/// 現行のフォーマット版。隠れ層の2つの幅を別々に持つ（ADR-0127）。
const FORMAT_VERSION: u32 = 3;
/// 隠れ層の幅を1つしか持たない旧版（L1とL2が同じ構成に限り読める）。
const FORMAT_VERSION_UNIFORM_HIDDEN: u32 = 2;

/// FNV-1a 64bit。重み列の破損検出用。
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

fn weight_bytes(net: &NnueNetwork) -> Vec<u8> {
    let mut v = Vec::new();
    for &x in &net.ft_b {
        v.extend_from_slice(&x.to_le_bytes());
    }
    for &x in &net.ft_w {
        v.extend_from_slice(&x.to_le_bytes());
    }
    for &x in &net.b2 {
        v.extend_from_slice(&x.to_le_bytes());
    }
    for &x in &net.w2 {
        v.push(x as u8);
    }
    for &x in &net.b3 {
        v.extend_from_slice(&x.to_le_bytes());
    }
    // w3はL1_PAD幅で持つが、ゼロ埋め列はファイルに残さない
    for row in net.w3.as_chunks::<L1_PAD>().0 {
        for &x in &row[..L1_OUT] {
            v.push(x as u8);
        }
    }
    for &x in &net.w4 {
        v.push(x as u8);
    }
    v.extend_from_slice(&net.b4.to_le_bytes());
    v
}

/// 学習来歴つきで書き出す。
pub fn save(net: &NnueNetwork, lineage: &str, w: &mut impl Write) -> std::io::Result<()> {
    let body = weight_bytes(net);
    w.write_all(MAGIC)?;
    w.write_all(&FORMAT_VERSION.to_le_bytes())?;
    for dim in [FT_IN as u32, FT_OUT as u32, L1_OUT as u32, L2_OUT as u32] {
        w.write_all(&dim.to_le_bytes())?;
    }
    let lb = lineage.as_bytes();
    w.write_all(&(lb.len() as u32).to_le_bytes())?;
    w.write_all(lb)?;
    w.write_all(&fnv1a(&body).to_le_bytes())?;
    w.write_all(&body)
}

fn read_u32(r: &mut impl Read) -> Result<u32, String> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)
        .map_err(|e| format!("読み込み失敗: {e}"))?;
    Ok(u32::from_le_bytes(b))
}

/// リトルエンディアンのバイト列リーダ。独自形式と互換ローダ
/// （nnue_compat）で共用する。
pub(crate) struct Cursor<'a> {
    body: &'a [u8],
    off: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(body: &'a [u8]) -> Self {
        Cursor { body, off: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .off
            .checked_add(n)
            .filter(|&e| e <= self.body.len())
            .ok_or_else(|| "重み列が短い".to_string())?;
        let s = &self.body[self.off..end];
        self.off = end;
        Ok(s)
    }

    pub(crate) fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub(crate) fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        self.take(n)
    }

    pub(crate) fn i16v(&mut self, n: usize) -> Result<Vec<i16>, String> {
        Ok(self
            .take(n * 2)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes(*c))
            .collect())
    }

    pub(crate) fn i32v(&mut self, n: usize) -> Result<Vec<i32>, String> {
        Ok(self
            .take(n * 4)?
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| i32::from_le_bytes(*c))
            .collect())
    }

    pub(crate) fn i8v(&mut self, n: usize) -> Result<Vec<i8>, String> {
        Ok(self.take(n)?.iter().map(|&b| b as i8).collect())
    }

    /// すべて読み切ったことを確認する。余りは構成不一致の兆候。
    pub(crate) fn expect_end(&self) -> Result<(), String> {
        let rest = self.body.len() - self.off;
        if rest != 0 {
            return Err(format!("末尾に余分な{rest}バイトがある"));
        }
        Ok(())
    }
}

/// ファイルの構成（FT入力・FT出力・L1・L2）。
type Dims = [usize; 4];

/// ヘッダを読み、(構成, 学習来歴, 検証済みの重み列) を返す。
fn read_header(r: &mut impl Read) -> Result<(Dims, String, Vec<u8>), String> {
    let mut magic = [0u8; 8];
    r.read_exact(&mut magic)
        .map_err(|e| format!("読み込み失敗: {e}"))?;
    if &magic != MAGIC {
        return Err("マジックが不一致（Himawari NNUE形式ではない）".to_string());
    }
    let version = read_u32(r)?;
    // 旧版は隠れ層の幅を1つしか書かない。L1とL2が同じとみなす
    let dims: Dims = match version {
        FORMAT_VERSION => [
            read_u32(r)? as usize,
            read_u32(r)? as usize,
            read_u32(r)? as usize,
            read_u32(r)? as usize,
        ],
        FORMAT_VERSION_UNIFORM_HIDDEN => {
            let d = [read_u32(r)?, read_u32(r)?, read_u32(r)?];
            [d[0] as usize, d[1] as usize, d[2] as usize, d[2] as usize]
        }
        other => return Err(format!("未対応のフォーマット版: {other}")),
    };
    let llen = read_u32(r)? as usize;
    if llen > 4096 {
        return Err("学習来歴が長すぎる".to_string());
    }
    let mut lb = vec![0u8; llen];
    r.read_exact(&mut lb)
        .map_err(|e| format!("読み込み失敗: {e}"))?;
    let lineage = String::from_utf8(lb).map_err(|_| "学習来歴がUTF-8でない".to_string())?;
    let mut hash_b = [0u8; 8];
    r.read_exact(&mut hash_b)
        .map_err(|e| format!("読み込み失敗: {e}"))?;
    let expect_hash = u64::from_le_bytes(hash_b);

    let mut body = Vec::new();
    r.read_to_end(&mut body)
        .map_err(|e| format!("読み込み失敗: {e}"))?;
    if fnv1a(&body) != expect_hash {
        return Err("重みハッシュが不一致（ファイル破損）".to_string());
    }
    Ok((dims, lineage, body))
}

/// 読み込む。戻り値は (ネットワーク, 学習来歴)。
pub fn load(r: &mut impl Read) -> Result<(NnueNetwork, String), String> {
    let (dims, lineage, body) = read_header(r)?;
    let expect = [FT_IN, FT_OUT, L1_OUT, L2_OUT];
    if dims != expect {
        return Err(format!(
            "アーキテクチャ不一致: ファイル{dims:?} 実装{expect:?}"
        ));
    }

    let mut cur = Cursor::new(&body);
    let ft_b = cur.i16v(FT_OUT)?;
    let ft_w = cur.i16v(FT_IN * FT_OUT)?;
    let b2 = cur.i32v(L1_OUT)?;
    let w2 = cur.i8v(L1_OUT * CONCAT)?;
    let b3 = cur.i32v(L2_OUT)?;
    let w3 = pad_l2_weights(&cur.i8v(L2_OUT * L1_OUT)?);
    let w4 = cur.i8v(L2_OUT)?;
    let b4 = cur.i32v(1)?[0];
    cur.expect_end()?;

    Ok((
        NnueNetwork {
            ft_w,
            ft_b,
            w2,
            b2,
            w3,
            b3,
            w4,
            b4,
        },
        lineage,
    ))
}

/// 別の構成のファイルを読み、いまのビルド構成へ合わせる（ADR-0127）。
///
/// **広げるときは評価値が元と完全に一致する。** 足した次元の重みと
/// バイアスをすべてゼロにすると、その出力はclipped ReLUで0になり、
/// 受け取る側の列もゼロなので積和に効かない。構成だけを変えて探索木を
/// 揃えられるので、速度の差だけを取り出せる。
///
/// 切り詰めるときは重みを捨てるので**評価値が変わる。** それでも重みの
/// 大きさは学習済みのままなので、活性が飽和した乱数ネットよりは現実に
/// 近い探索木になる。小さい構成を基準にして比べたいときに使う。
pub fn load_resized(r: &mut impl Read) -> Result<(NnueNetwork, String), String> {
    let (dims, lineage, body) = read_header(r)?;
    let [ft_in, src_ft, src_l1, src_l2] = dims;
    if ft_in != FT_IN {
        return Err(format!("FT入力が違う: ファイル{ft_in} 実装{FT_IN}"));
    }

    /// 行優先の行列を、行数と列幅を合わせて写す。余りはゼロのまま。
    fn fit_rows<T: Copy + Default>(
        src: &[T],
        src_cols: usize,
        dst_rows: usize,
        dst_cols: usize,
        used_cols: usize,
    ) -> Vec<T> {
        let mut v = vec![T::default(); dst_rows * dst_cols];
        let cols = src_cols.min(used_cols);
        for (i, row) in src.chunks_exact(src_cols).take(dst_rows).enumerate() {
            v[i * dst_cols..i * dst_cols + cols].copy_from_slice(&row[..cols]);
        }
        v
    }

    /// 長さを合わせる。伸ばすぶんはゼロで埋める。
    fn fit<T: Copy + Default>(mut src: Vec<T>, len: usize) -> Vec<T> {
        src.resize(len, T::default());
        src
    }

    let mut cur = Cursor::new(&body);
    let ft_b = fit(cur.i16v(src_ft)?, FT_OUT);
    let ft_w = fit_rows(&cur.i16v(FT_IN * src_ft)?, src_ft, FT_IN, FT_OUT, FT_OUT);
    let b2 = fit(cur.i32v(src_l1)?, L1_OUT);
    // 隠れ層1の入力は2視点の連結なので、視点ごとに写す
    let src_w2 = cur.i8v(src_l1 * src_ft * 2)?;
    let mut w2 = vec![0i8; L1_OUT * CONCAT];
    let ft_cols = src_ft.min(FT_OUT);
    for (o, row) in src_w2.chunks_exact(src_ft * 2).take(L1_OUT).enumerate() {
        for half in 0..2 {
            let dst = o * CONCAT + half * FT_OUT;
            w2[dst..dst + ft_cols].copy_from_slice(&row[half * src_ft..half * src_ft + ft_cols]);
        }
    }
    let b3 = fit(cur.i32v(src_l2)?, L2_OUT);
    // 列幅はL1_PADだが、値を置くのはL1_OUTまで（残りは常にゼロ）
    let w3 = fit_rows(&cur.i8v(src_l2 * src_l1)?, src_l1, L2_OUT, L1_PAD, L1_OUT);
    let w4 = fit(cur.i8v(src_l2)?, L2_OUT);
    let b4 = cur.i32v(1)?[0];
    cur.expect_end()?;

    Ok((
        NnueNetwork {
            ft_w,
            ft_b,
            w2,
            b2,
            w3,
            b3,
            w4,
            b4,
        },
        format!("{lineage} / resized to {ARCH}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nnue::evaluate_scalar;
    use himawari_core::{Position, SFEN_STARTPOS};

    /// 乱数ネットのroundtrip（書き出し→読み込み→評価一致）。
    #[test]
    fn roundtrip() {
        let net = NnueNetwork::random(99);
        let mut buf = Vec::new();
        save(&net, "test-lineage seed=99", &mut buf).unwrap();
        let (loaded, lineage) = load(&mut buf.as_slice()).unwrap();
        assert_eq!(lineage, "test-lineage seed=99");
        let pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
        assert_eq!(evaluate_scalar(&net, &pos), evaluate_scalar(&loaded, &pos));
        assert_eq!(net.ft_w, loaded.ft_w);
        assert_eq!(net.w4, loaded.w4);
        assert_eq!(net.b4, loaded.b4);
        // 隠れ層2はL1_PAD幅のまま往復する（ゼロ埋め列も含めて一致）
        assert_eq!(net.w3, loaded.w3);
        assert_eq!(net.w3.len(), crate::nnue::L2_OUT * L1_PAD);
    }

    /// 版2のファイル（隠れ層の幅が1つ）も読める。L1とL2が同じ構成に限る。
    #[test]
    fn version2_is_readable_when_hidden_widths_match() {
        let net = NnueNetwork::random(123);
        let mut v3 = Vec::new();
        save(&net, "v2互換の検査", &mut v3).unwrap();
        // 版3のヘッダから版番号とL2の次元を落として版2の並びにする
        let mut v2 = Vec::new();
        v2.extend_from_slice(&v3[..8]);
        v2.extend_from_slice(&FORMAT_VERSION_UNIFORM_HIDDEN.to_le_bytes());
        v2.extend_from_slice(&v3[12..24]); // FT_IN・FT_OUT・L1_OUT
        v2.extend_from_slice(&v3[28..]); // L2_OUTを飛ばして以降すべて

        let result = load(&mut v2.as_slice());
        if L1_OUT == L2_OUT {
            let (loaded, lineage) = result.unwrap();
            assert_eq!(lineage, "v2互換の検査");
            assert_eq!(net.ft_w, loaded.ft_w);
            assert_eq!(net.w3, loaded.w3);
        } else {
            assert!(result.is_err(), "幅が違う構成で版2を受け入れてはいけない");
        }
    }

    /// ヘッダ・重みの破損を検出してエラーになる。
    #[test]
    fn corruption_is_detected() {
        let net = NnueNetwork::random(7);
        let mut buf = Vec::new();
        save(&net, "", &mut buf).unwrap();
        // マジック破壊
        let mut bad = buf.clone();
        bad[0] ^= 0xFF;
        assert!(load(&mut bad.as_slice()).is_err());
        // 重み1バイト破壊（ハッシュ検出）
        let mut bad = buf.clone();
        let last = bad.len() - 1;
        bad[last] ^= 0x01;
        assert!(load(&mut bad.as_slice()).is_err());
        // 末尾切り捨て
        let bad = &buf[..buf.len() - 8];
        assert!(load(&mut &bad[..]).is_err());
    }
}

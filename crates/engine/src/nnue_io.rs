//! NNUE評価ファイルの独自フォーマット（ADR-0037）。
//!
//! ヘッダ: マジック・フォーマット版・アーキテクチャ記述子
//! （各塔の次元）・学習来歴文字列・重みハッシュ。以降は
//! リトルエンディアンの重み列。アーキテクチャ不一致・ハッシュ
//! 不一致は読み込みエラーにする（気づかず壊れたネットで
//! 対局する事故を防ぐ）。

use std::io::{Read, Write};

use crate::nnue::{CONCAT, EFFECT_IN, EFFECT_OUT, FT_IN, FT_OUT, HIDDEN, NnueNetwork};

const MAGIC: &[u8; 8] = b"HMWRNNUE";
const FORMAT_VERSION: u32 = 1;

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
    for &x in &net.ef_b {
        v.extend_from_slice(&x.to_le_bytes());
    }
    for &x in &net.ef_w {
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
    for &x in &net.w3 {
        v.push(x as u8);
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
    for dim in [
        FT_IN as u32,
        FT_OUT as u32,
        EFFECT_IN as u32,
        EFFECT_OUT as u32,
        HIDDEN as u32,
    ] {
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

/// 読み込む。戻り値は (ネットワーク, 学習来歴)。
pub fn load(r: &mut impl Read) -> Result<(NnueNetwork, String), String> {
    let mut magic = [0u8; 8];
    r.read_exact(&mut magic)
        .map_err(|e| format!("読み込み失敗: {e}"))?;
    if &magic != MAGIC {
        return Err("マジックが不一致（Himawari NNUE形式ではない）".to_string());
    }
    let version = read_u32(r)?;
    if version != FORMAT_VERSION {
        return Err(format!("未対応のフォーマット版: {version}"));
    }
    let dims = [
        read_u32(r)?,
        read_u32(r)?,
        read_u32(r)?,
        read_u32(r)?,
        read_u32(r)?,
    ];
    let expect = [
        FT_IN as u32,
        FT_OUT as u32,
        EFFECT_IN as u32,
        EFFECT_OUT as u32,
        HIDDEN as u32,
    ];
    if dims != expect {
        return Err(format!(
            "アーキテクチャ不一致: ファイル{dims:?} 実装{expect:?}"
        ));
    }
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

    let mut cur = Cursor::new(&body);
    let ft_b = cur.i16v(FT_OUT)?;
    let ft_w = cur.i16v(FT_IN * FT_OUT)?;
    let ef_b = cur.i16v(EFFECT_OUT)?;
    let ef_w = cur.i16v(EFFECT_IN * EFFECT_OUT)?;
    let b2 = cur.i32v(HIDDEN)?;
    let w2 = cur.i8v(HIDDEN * CONCAT)?;
    let b3 = cur.i32v(HIDDEN)?;
    let w3 = cur.i8v(HIDDEN * HIDDEN)?;
    let w4 = cur.i8v(HIDDEN)?;
    let b4 = cur.i32v(1)?[0];
    cur.expect_end()?;

    Ok((
        NnueNetwork {
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
        },
        lineage,
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

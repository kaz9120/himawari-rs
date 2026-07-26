//! やねうら王形式NNUE（nn.bin）の互換ローダ（ADR-0037）。
//!
//! HalfKP 256×2-32-32-1の公開評価関数を「第1塔＋利き塔ゼロ」として
//! 読み込む。用途は推論スタックの外部照合と、学習のウォーム
//! スタート初期化（ADR-0034）に限る。互換形式での書き出しはしない。
//!
//! ファイル構成（Stockfish系NNUEと同一）:
//! version u32 / 全体ハッシュ u32 / アーキテクチャ文字列(長さ u32 + 本体) /
//! FTヘッダ u32 / FTバイアス i16[256] / FT重み i16[125388×256] /
//! ネットワークヘッダ u32 / 隠れ1: バイアス i32[32]+重み i8[32×512] /
//! 隠れ2: バイアス i32[32]+重み i8[32×32] / 出力: バイアス i32[1]+重み i8[32]。
//! すべてリトルエンディアン。重みの並びは本実装と同じ
//! （FTは特徴行×出力列、隠れ層は出力行×入力列）。

use std::io::Read;

use crate::nnue::{CONCAT, FT_IN, FT_OUT, HIDDEN, NnueNetwork};
use crate::nnue_io::Cursor;

/// 標準NNUEのフォーマットバージョン（Stockfish系・やねうら王共通）。
const NN_BIN_VERSION: u32 = 0x7AF3_2F16;
/// nn.binを読み込む。戻り値は (ネットワーク, アーキテクチャ文字列)。
/// ハッシュ値は検証しない（構成の不一致は次元とバイト数の照合で
/// 検出し、重みの正しさは外部のevaluate値照合で確認する）。
pub fn load_nn_bin(r: &mut impl Read) -> Result<(NnueNetwork, String), String> {
    let mut data = Vec::new();
    r.read_to_end(&mut data)
        .map_err(|e| format!("読み込み失敗: {e}"))?;
    let mut cur = Cursor::new(&data);

    let version = cur.u32()?;
    if version != NN_BIN_VERSION {
        return Err(format!(
            "nn.binのバージョンが不一致: 0x{version:08X} (期待 0x{NN_BIN_VERSION:08X})"
        ));
    }
    let _file_hash = cur.u32()?;
    let alen = cur.u32()? as usize;
    if alen > 1024 {
        return Err("アーキテクチャ文字列が長すぎる".to_string());
    }
    let arch = String::from_utf8(cur.bytes(alen)?.to_vec())
        .map_err(|_| "アーキテクチャ文字列がUTF-8でない".to_string())?;
    if !arch.contains("HalfKP") {
        return Err(format!("HalfKPネットではない: {arch}"));
    }
    // nn.binのFT次元は256で固定である（ADR-0067）
    if FT_OUT != 256 {
        return Err(format!(
            "nn.binはFT 256専用。このビルドはFT {FT_OUT}（ADR-0067）"
        ));
    }

    let _ft_hash = cur.u32()?;
    let ft_b = cur.i16v(FT_OUT)?;
    let ft_w = cur.i16v(FT_IN * FT_OUT)?;

    let _net_hash = cur.u32()?;
    let b2 = cur.i32v(HIDDEN)?;
    let w2 = cur.i8v(HIDDEN * CONCAT)?;
    let b3 = cur.i32v(HIDDEN)?;
    let w3 = cur.i8v(HIDDEN * HIDDEN)?;
    let b4 = cur.i32v(1)?[0];
    let w4 = cur.i8v(HIDDEN)?;
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
        arch,
    ))
}

// テストは256のnn.binを組み立てるため、512ビルドでは回さない
#[cfg(all(test, not(feature = "ft512")))]
mod tests {
    use super::*;
    use crate::nnue::evaluate_scalar;
    use himawari_core::{MoveList, Position, SFEN_STARTPOS, generate_legal};

    /// ネットをnn.bin形式に書き出す（テスト専用の逆関数）。
    fn to_nn_bin(net: &NnueNetwork, arch: &str) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend(NN_BIN_VERSION.to_le_bytes());
        v.extend(0u32.to_le_bytes()); // 全体ハッシュ（検証対象外）
        v.extend((arch.len() as u32).to_le_bytes());
        v.extend(arch.as_bytes());
        v.extend(0u32.to_le_bytes()); // FTヘッダ
        for &x in &net.ft_b {
            v.extend(x.to_le_bytes());
        }
        for &x in &net.ft_w {
            v.extend(x.to_le_bytes());
        }
        v.extend(0u32.to_le_bytes()); // ネットワークヘッダ
        for &x in &net.b2 {
            v.extend(x.to_le_bytes());
        }
        for &x in &net.w2 {
            v.push(x as u8);
        }
        for &x in &net.b3 {
            v.extend(x.to_le_bytes());
        }
        for &x in &net.w3 {
            v.push(x as u8);
        }
        v.extend(net.b4.to_le_bytes());
        for &x in &net.w4 {
            v.push(x as u8);
        }
        v
    }

    /// 書き出し→読み込みで全重みと評価値が一致する（レイアウト検証）。
    #[test]
    fn nn_bin_roundtrip() {
        let net = NnueNetwork::random(2026);
        let buf = to_nn_bin(&net, "Features=HalfKP(Friend)[125388->256x2]");
        let (loaded, arch) = load_nn_bin(&mut buf.as_slice()).unwrap();
        assert!(arch.contains("HalfKP"));
        assert_eq!(net.ft_w, loaded.ft_w);
        assert_eq!(net.ft_b, loaded.ft_b);
        assert_eq!(net.w2, loaded.w2);
        assert_eq!(net.b2, loaded.b2);
        assert_eq!(net.w3, loaded.w3);
        assert_eq!(net.b3, loaded.b3);
        assert_eq!(net.w4, loaded.w4);
        assert_eq!(net.b4, loaded.b4);

        let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
        for _ in 0..10 {
            assert_eq!(evaluate_scalar(&net, &pos), evaluate_scalar(&loaded, &pos));
            let mut list = MoveList::default();
            generate_legal(&pos, true, &mut list);
            pos.do_move(list.as_slice()[0]);
        }
    }

    /// 壊れた入力をエラーにする。
    #[test]
    fn nn_bin_rejects_bad_input() {
        let net = NnueNetwork::random(3);
        let buf = to_nn_bin(&net, "Features=HalfKP(Friend)[125388->256x2]");
        // バージョン不一致
        let mut bad = buf.clone();
        bad[0] ^= 0xFF;
        assert!(load_nn_bin(&mut bad.as_slice()).is_err());
        // HalfKPでないアーキテクチャ
        let net2 = NnueNetwork::random(3);
        let bad = to_nn_bin(&net2, "Features=K-P[...]");
        assert!(load_nn_bin(&mut bad.as_slice()).is_err());
        // 末尾切り捨て・余分バイト
        assert!(load_nn_bin(&mut &buf[..buf.len() - 1]).is_err());
        let mut bad = buf.clone();
        bad.push(0);
        assert!(load_nn_bin(&mut bad.as_slice()).is_err());
    }

    /// 公開評価関数との外部照合（ADR-0037。ローカル実行、CIではスキップ）。
    ///
    /// 環境変数 HIMAWARI_NN_BIN にnn.binのパスを渡すと実行する。
    /// さらに HIMAWARI_NN_REF に「SFEN,評価値」のCSV（#行はコメント）を
    /// 渡すと、各局面のevaluate値の完全一致を検査する。参照値は
    /// やねうら王など既存実装のevalコマンドで生成する（手番視点・
    /// FV_SCALE=16適用後の整数値）。
    #[test]
    fn nn_bin_external_reference() {
        let Ok(path) = std::env::var("HIMAWARI_NN_BIN") else {
            eprintln!("HIMAWARI_NN_BIN未設定のためスキップ");
            return;
        };
        let mut f = std::fs::File::open(&path).expect("nn.binを開けない");
        let (net, arch) = load_nn_bin(&mut f).expect("nn.binの読み込みに失敗");
        eprintln!("読み込み成功: {arch}");

        let pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
        let v = evaluate_scalar(&net, &pos);
        eprintln!("startpos eval = {v}");
        assert!(v.abs() < 1000, "強いネットの初期局面評価としては異常: {v}");

        let Ok(ref_path) = std::env::var("HIMAWARI_NN_REF") else {
            eprintln!("HIMAWARI_NN_REF未設定のため読み込み確認のみで終了");
            return;
        };
        let text = std::fs::read_to_string(&ref_path).expect("参照CSVを開けない");
        let mut checked = 0;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (sfen, expect) = line.rsplit_once(',').expect("形式は SFEN,評価値");
            let expect: i32 = expect.trim().parse().expect("評価値が整数でない");
            let pos = Position::from_sfen(sfen.trim()).expect("SFENが不正");
            assert_eq!(
                evaluate_scalar(&net, &pos),
                expect,
                "評価値が参照実装と不一致: {sfen}"
            );
            checked += 1;
        }
        assert!(checked > 0, "参照CSVに局面がない");
        eprintln!("{checked}局面すべて一致");
    }
}

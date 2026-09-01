//! 計測に使う固定局面（ADR-0074・ADR-0122）。
//!
//! かつて `verify-feature.sh`・`bench-nps.sh`・`profile.sh` の3本が同じ
//! 4局面を各自に持っていた。片方だけ足すと条件がずれるため1か所に置く。

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// 検証局面。初期局面と `openings/start_sfens_ply24.txt` の先頭3行。
/// 固定して条件を揃える。増やすときは末尾へ足し、既存の並びは変えない。
/// 文字列はUSIの `position` に続けて渡せる形で持つ。
pub const POSITIONS: [&str; 4] = [
    "startpos",
    "sfen +Bn1g2s1l/2skg2r1/ppppp1n1p/5bpp1/5p1P1/2P6/PP1PP1P1P/1SK2S1R1/LN1G1G1NL w Lp 24",
    "sfen +R1G4nl/1g4+Ss1/1kspp2p1/ppp2pS1p/4n4/P4Gp1P/1P1PP1P2/1+n2K2R1/7NL w G2P2b2lp 24",
    "sfen 1n1gk2nl/1Bsr3s1/lp2ppgpp/p1pp2p2/7P1/P1P6/1PNPPPP1P/1SKG2SR1/L4G1NL w b 24",
];

/// 局面ごとの深さ補正。局面3だけ枝が広く、同じ深さでは時間を独占する。
/// NPS計測とプロファイルで使う（機能検証は全局面を同じ深さで読む）。
pub const DEPTH_ADJUST: [i32; POSITIONS.len()] = [0, 0, -3, 0];

/// 局面indexで補正した深さ。深さ1を下回らせない。
pub fn depth_at(base: u32, index: usize) -> u32 {
    let adjusted = base as i32 + DEPTH_ADJUST[index];
    adjusted.max(1) as u32
}

/// 局面リストをファイルから読む（ADR-0186）。1行が `position` に続けて
/// 渡せる形の文字列で、`#` で始まる行と空行は読み飛ばす。
///
/// 組み込みの4局面はすべてSFENで、履歴を持たない。履歴に依存する処理
/// （千日手・優等局面の判定）は手順つきの局面でしか測れないので、
/// `sfen <局面> moves <手順>` の形を外から渡せるようにする。
pub fn read_positions(path: &Path) -> Result<Vec<String>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("局面リストを読めない: {}", path.display()))?;
    let positions: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect();
    if positions.is_empty() {
        bail!("局面リストが空だ: {}", path.display());
    }
    Ok(positions)
}

/// 組み込みの4局面を、ファイル読み込みと同じ型で返す。
pub fn builtin_positions() -> Vec<String> {
    POSITIONS.iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR-0074の「3局面以上」を満たし、補正が局面と1対1で対応する。
    #[test]
    fn positions_and_adjustments_line_up() {
        assert_eq!(POSITIONS.len(), 4);
        assert_eq!(DEPTH_ADJUST.len(), POSITIONS.len());
        assert_eq!(POSITIONS[0], "startpos");
        for pos in &POSITIONS[1..] {
            assert!(pos.starts_with("sfen "), "SFENの前置きが要る: {pos}");
        }
    }

    #[test]
    fn depth_at_applies_adjustment() {
        assert_eq!(depth_at(19, 0), 19);
        assert_eq!(depth_at(19, 2), 16);
        assert_eq!(depth_at(25, 2), 22);
        // 浅い深さでも0や負にしない
        assert_eq!(depth_at(1, 2), 1);
    }
}

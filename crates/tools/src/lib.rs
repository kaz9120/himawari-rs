//! 開発用ツールの共有部分（ADR-0122）。
//!
//! USIエンジンを起動して測るbin（`bench`・`verify`・`profile`）と
//! `selfplay` が、エンジンクライアントと検証局面をここで共有する。
//! かつて3本のshellが同じ起動処理を各自に持ち、後始末を落としていた。

pub mod csa;
pub mod game;
pub mod positions;
pub mod stop_file;
pub mod usi_engine;

use std::path::{Path, PathBuf};

/// 終了コードの規約（ADR-0122）。0は成功。
pub mod exit {
    /// 判定結果。正常に測れたうえで「進むな」を意味する。
    pub const JUDGEMENT: u8 = 1;
    /// 引数エラー。
    pub const USAGE: u8 = 2;
    /// 実行時エラー。
    pub const RUNTIME: u8 = 3;
}

/// 引数エラーで終わる（ADR-0122の終了コード2）。
pub fn usage_error(msg: &str) -> ! {
    eprintln!("エラー: {msg}");
    std::process::exit(exit::USAGE as i32)
}

/// `Result<T, String>` をanyhowへ載せ替える。`usi_engine` は `selfplay`
/// 由来でエラー型がStringのため、binごとに変換を書かずに済ませる。
pub trait OrBail<T> {
    fn or_bail(self) -> anyhow::Result<T>;
}

impl<T> OrBail<T> for Result<T, String> {
    fn or_bail(self) -> anyhow::Result<T> {
        self.map_err(anyhow::Error::msg)
    }
}

/// 3桁ごとにカンマを入れる。表の桁を目で追えるようにする。
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// NPS。深さが浅いと1ミリ秒未満で読み終わるため、ms=0を明示的に弾く。
pub fn nps(nodes: u64, ms: u64) -> u64 {
    if ms == 0 {
        return 0;
    }
    nodes.saturating_mul(1000) / ms
}

/// 変化率を `+1.23%` の形にする。基準が0なら比を取れないので `n/a`。
pub fn percent_delta(base: f64, value: f64, decimals: usize) -> String {
    if base == 0.0 {
        return "n/a".to_string();
    }
    format!("{:+.*}%", decimals, (value - base) * 100.0 / base)
}

/// 実行できるファイルか確かめる。数分かかる計測を始める前に落とす。
pub fn ensure_executable(path: &Path) -> anyhow::Result<()> {
    let meta = std::fs::metadata(path)
        .map_err(|e| anyhow::anyhow!("実行できない: {} ({e})", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if !meta.is_file() || meta.permissions().mode() & 0o111 == 0 {
            anyhow::bail!("実行できない: {}", path.display());
        }
    }
    #[cfg(not(unix))]
    if !meta.is_file() {
        anyhow::bail!("実行できない: {}", path.display());
    }
    Ok(())
}

/// エンジンのパスを文字列で取る。`usi_engine` が `&str` を受けるため。
pub fn path_str(path: &Path) -> anyhow::Result<&str> {
    path.to_str()
        .ok_or_else(|| anyhow::anyhow!("パスがUTF-8でない: {}", path.display()))
}

/// 評価関数のパスを決める。明示指定がなければ環境変数 `EVAL_FILE` を読む。
/// `scripts/env.sh` がexportする値をそのまま使う契約にして、shell側と
/// 疎結合にする（ADR-0122）。
pub fn eval_file(explicit: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    let path = match explicit {
        Some(p) => p,
        None => match std::env::var_os("EVAL_FILE") {
            Some(v) => PathBuf::from(v),
            None => usage_error(
                "評価関数が決まらない。scripts/env.sh を source するか --eval-file で渡す",
            ),
        },
    };
    if !path.is_file() {
        anyhow::bail!("評価関数がない: {}", path.display());
    }
    Ok(path)
}

/// エンジンへ渡すsetoption。全binで1スレッド固定にして、ノード数と
/// NPSを再現できるようにする。
pub fn single_thread_options(eval: &Path) -> Vec<(String, String)> {
    vec![
        ("EvalFile".to_string(), eval.display().to_string()),
        ("Threads".to_string(), "1".to_string()),
    ]
}

/// 表示用のファイル名。パスが長いと表が読めなくなる。
pub fn basename(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_inserts_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(12_345_678), "12,345,678");
    }

    /// 旧 bench-nps.sh はms=0でゼロ除算した（ADR-0122）。
    #[test]
    fn nps_survives_zero_elapsed() {
        assert_eq!(nps(1000, 0), 0);
        assert_eq!(nps(0, 0), 0);
        assert_eq!(nps(1_000_000, 1000), 1_000_000);
        // 桁あふれで落ちない（saturating）
        assert_eq!(nps(u64::MAX, 1), u64::MAX);
    }

    #[test]
    fn percent_delta_handles_zero_base() {
        assert_eq!(percent_delta(0.0, 10.0, 2), "n/a");
        assert_eq!(percent_delta(100.0, 101.0, 2), "+1.00%");
        assert_eq!(percent_delta(100.0, 90.0, 0), "-10%");
    }
}

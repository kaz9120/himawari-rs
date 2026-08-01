//! 長時間走る処理を安全に止める仕組み（ADR-0123）。
//!
//! SPRTは数時間、定跡生成は十数時間かかる。その間マシンは占有される。
//! 別の作業へCPUを回したくなったとき、`pkill` で殺すと最後の保存から
//! 先が消える。
//!
//! 停止ファイルが現れたら、切りのよいところで保存して終わる。
//!
//! ```text
//! touch data/book/main.db.stop
//! ```
//!
//! シグナルを捕まえる案もあるが、止めたい処理は `nohup ... &` で走らせて
//! いるのでCtrl-Cが届かない。どのみちPIDを調べることになる。ファイルなら
//! 依存も要らない（ADR-0123の案A・案Bの比較）。

use std::path::{Path, PathBuf};

/// 停止の要求を受け取る口。
pub struct StopFile {
    path: PathBuf,
}

impl StopFile {
    /// 主な出力の隣に置く（`<出力>.stop`）。出力のパスさえ分かれば
    /// 止め方が分かる形にする。
    pub fn beside(output: &Path) -> Self {
        let mut name = output.as_os_str().to_os_string();
        name.push(".stop");
        Self {
            path: PathBuf::from(name),
        }
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 前回の残りを片付ける。`pkill` で殺されると消せずに残り、次回が
    /// 即終了する。起動時に黙って消すと気づけないので、知らせてから消す。
    pub fn clear_stale(&self) {
        if self.path.exists() {
            eprintln!(
                "前回の停止ファイルが残っている。消して続ける: {}",
                self.path.display()
            );
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// 停止を求められているか。1単位の処理に入る前に呼ぶ。
    pub fn requested(&self) -> bool {
        self.path.exists()
    }

    /// 停止して終わるときに呼ぶ。消しておかないと、再開したつもりの
    /// 次回が何もせずに終わる。
    pub fn consume(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テストごとに別のディレクトリを使う。並列実行で踏み合わないため。
    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("himawari-stopfile-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn beside_appends_suffix_to_the_whole_name() {
        let sf = StopFile::beside(Path::new("data/book/main.db"));
        assert_eq!(sf.path(), Path::new("data/book/main.db.stop"));
    }

    #[test]
    fn requested_follows_the_file() {
        let dir = temp_dir("requested");
        let sf = StopFile::at(dir.join("x.stop"));
        assert!(!sf.requested());
        std::fs::write(sf.path(), "").expect("touch");
        assert!(sf.requested());
        sf.consume();
        assert!(!sf.requested());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// pkillで殺されて残った停止ファイルを、次回起動が片付けること。
    /// 片付けないと再開したつもりの次回が何もせず終わる。
    #[test]
    fn clear_stale_removes_leftover() {
        let dir = temp_dir("stale");
        let sf = StopFile::at(dir.join("y.stop"));
        std::fs::write(sf.path(), "").expect("touch");
        sf.clear_stale();
        assert!(!sf.requested());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn consume_is_quiet_when_absent() {
        let dir = temp_dir("absent");
        let sf = StopFile::at(dir.join("z.stop"));
        sf.consume();
        sf.clear_stale();
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}

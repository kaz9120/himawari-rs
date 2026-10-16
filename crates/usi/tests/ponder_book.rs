//! ponderと定跡の組み合わせで、bestmoveが1回のgoにちょうど1本出ることを
//! 固定する（ADR-0231、Issue #747）。
//!
//! go ponderでは定跡を引かずに探索し、ponderhitで定跡を引く。ヒットしたら
//! 探索の結論を捨てて定跡の手を出す。二重のbestmoveを出さないことが要点で
//! ある（Issue #545の類型）。入力を一度に流し、quitで終わるまで待つので、
//! 判定はマシンの速度に依存しない。

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use himawari_engine::nnue::NnueNetwork;

/// 乱数ネットと1局面だけの定跡を置いた作業ディレクトリ。
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let dir =
            std::env::temp_dir().join(format!("himawari-ponderbook-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create_dir_all");
        let mut f = std::fs::File::create(dir.join("net.hmwr")).expect("create net");
        himawari_engine::nnue_io::save(&NnueNetwork::random(7).finish(), "test", &mut f)
            .expect("save net");
        // 平手の初期局面だけを持つ定跡。7g7fを指す
        std::fs::write(
            dir.join("book.db"),
            "#YANEURAOU-DB2016 1.00\n\
             sfen lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1\n\
             7g7f none 0 32 1\n",
        )
        .expect("write book");
        Fixture { dir }
    }

    /// 初期化から始めて `body` を流し、quitで終えた出力の行を返す。
    fn run(&self, body: &str) -> Vec<String> {
        let input = format!(
            "usi\n\
             setoption name USI_Hash value 16\n\
             setoption name EvalFile value {}\n\
             setoption name BookFile value {}\n\
             isready\n\
             usinewgame\n\
             {body}\
             quit\n",
            self.dir.join("net.hmwr").display(),
            self.dir.join("book.db").display(),
        );
        let mut child = Command::new(env!("CARGO_BIN_EXE_himawari"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn himawari");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(input.as_bytes())
            .expect("write stdin");
        let out = child.wait_with_output().expect("wait himawari");
        assert_eq!(out.status.code(), Some(0));
        String::from_utf8(out.stdout)
            .expect("utf-8")
            .lines()
            .map(str::to_string)
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn bestmoves(lines: &[String]) -> Vec<&str> {
    lines
        .iter()
        .filter(|l| l.starts_with("bestmove"))
        .map(String::as_str)
        .collect()
}

fn has_search_info(lines: &[String]) -> bool {
    lines.iter().any(|l| l.starts_with("info depth"))
}

fn book_hits(lines: &[String]) -> usize {
    lines.iter().filter(|l| l.contains("book hit")).count()
}

const TIME: &str = "btime 1000 wtime 1000 byoyomi 1000";

/// ponderなしのgoは、従来どおり探索せずに定跡の手を返す。
#[test]
fn plain_go_on_book_position_returns_book_move_without_search() {
    let fx = Fixture::new("plain");
    let lines = fx.run(&format!("position startpos\ngo {TIME}\n"));
    let bm = bestmoves(&lines);
    assert_eq!(bm.len(), 1, "{lines:#?}");
    assert!(bm[0].starts_with("bestmove 7g7f"), "{lines:#?}");
    assert!(!has_search_info(&lines), "{lines:#?}");
}

/// 定跡にある局面でgo ponder→ponderhit。ponder中は探索し、ponderhitで
/// 定跡の手を1本だけ出す。探索の結論は出さない。
#[test]
fn ponderhit_on_book_position_returns_only_the_book_move() {
    let fx = Fixture::new("hit");
    let lines = fx.run(&format!("position startpos\ngo ponder {TIME}\nponderhit\n"));
    let bm = bestmoves(&lines);
    assert_eq!(bm.len(), 1, "{lines:#?}");
    assert!(bm[0].starts_with("bestmove 7g7f"), "{lines:#?}");
    assert_eq!(book_hits(&lines), 1, "{lines:#?}");
    // ponder中に探索した（depth 1は止める前に必ず終える）
    assert!(has_search_info(&lines), "{lines:#?}");
    // 定跡を引いた後にbestmoveを出す
    let hit_at = lines.iter().position(|l| l.contains("book hit")).unwrap();
    let bm_at = lines
        .iter()
        .position(|l| l.starts_with("bestmove"))
        .unwrap();
    assert!(hit_at < bm_at, "{lines:#?}");
}

/// ponderが外れた（stop）。定跡は引かず、探索の結論を1本だけ出す。
/// GUIはこのbestmoveを受けてから次のgoを送るので、続くgoは
/// ponderなしのgoのテストが受け持つ。
#[test]
fn stop_after_ponder_on_book_position_returns_search_result() {
    let fx = Fixture::new("stop");
    let lines = fx.run(&format!("position startpos\ngo ponder {TIME}\nstop\n"));
    let bm = bestmoves(&lines);
    assert_eq!(bm.len(), 1, "{lines:#?}");
    assert_eq!(book_hits(&lines), 0, "{lines:#?}");
    assert!(has_search_info(&lines), "{lines:#?}");
    assert!(!bm[0].starts_with("bestmove resign"), "{lines:#?}");
}

/// 定跡にない局面でgo ponder→ponderhit。従来どおり探索を続け、
/// その結論を1本だけ出す。
#[test]
fn ponderhit_off_book_returns_search_result() {
    let fx = Fixture::new("offbook");
    let lines = fx.run("position startpos moves 2g2f\ngo ponder depth 3\nponderhit\n");
    let bm = bestmoves(&lines);
    assert_eq!(bm.len(), 1, "{lines:#?}");
    assert_eq!(book_hits(&lines), 0, "{lines:#?}");
    assert!(has_search_info(&lines), "{lines:#?}");
    assert!(!bm[0].starts_with("bestmove resign"), "{lines:#?}");
}

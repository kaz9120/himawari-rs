//! 定跡の読み込みと参照（ADR-0063）。
//!
//! 形式はやねうら王のdb形式互換。`sfen` 行に続けて候補手を並べる。
//!
//! ```text
//! #YANEURAOU-DB2016 1.00
//! sfen lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1
//! 7g7f 3c3d 42 24 1
//! 2g2f 8c8d 38 24 1
//! ```
//!
//! 各行は `指し手 相手の予想応手 評価値 探索深さ 出現回数`。
//! キーは手数を除いたsfen文字列で、局面の同一性を手数に依存させない。

use std::collections::HashMap;

use himawari_core::Position;

pub struct BookEntry {
    pub mv: String,
    pub value: i32,
}

#[derive(Default)]
pub struct Book {
    entries: HashMap<String, Vec<BookEntry>>,
}

/// 手数を除いたsfen（盤面・手番・手駒）をキーにする。
pub fn book_key(pos: &Position) -> String {
    let sfen = pos.to_sfen();
    match sfen.rfind(' ') {
        Some(i) => sfen[..i].to_string(),
        None => sfen,
    }
}

impl Book {
    pub fn load(path: &str) -> std::io::Result<Book> {
        let text = std::fs::read_to_string(path)?;
        let mut entries: HashMap<String, Vec<BookEntry>> = HashMap::new();
        let mut key: Option<String> = None;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("sfen ") {
                // 手数が付いていれば落とす（db形式は手数1で書くのが慣例）
                let f: Vec<&str> = rest.split_whitespace().collect();
                key = Some(if f.len() >= 4 {
                    f[..3].join(" ")
                } else {
                    f.join(" ")
                });
                continue;
            }
            let Some(k) = &key else { continue };
            // 指し手 予想応手 評価値 深さ 出現回数
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.is_empty() {
                continue;
            }
            let value = f.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            entries.entry(k.clone()).or_default().push(BookEntry {
                mv: f[0].to_string(),
                value,
            });
        }
        Ok(Book { entries })
    }

    pub fn positions(&self) -> usize {
        self.entries.len()
    }

    /// 評価値が最大の手を返す（ADR-0063: 決定的に選ぶ）。
    pub fn probe(&self, pos: &Position) -> Option<&BookEntry> {
        self.entries
            .get(&book_key(pos))?
            .iter()
            .max_by_key(|e| e.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use himawari_core::SFEN_STARTPOS;

    fn write_tmp(name: &str, body: &str) -> String {
        let mut p = std::env::temp_dir();
        p.push(name);
        std::fs::write(&p, body).expect("write");
        p.to_string_lossy().to_string()
    }

    #[test]
    fn probe_picks_highest_value() {
        let pos = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
        let body = format!(
            "#YANEURAOU-DB2016 1.00\nsfen {}\n7g7f 3c3d 42 24 1\n2g2f 8c8d 55 24 1\n",
            book_key(&pos)
        );
        let path = write_tmp("himawari_book_test1.db", &body);
        let book = Book::load(&path).expect("load");
        assert_eq!(book.positions(), 1);
        assert_eq!(book.probe(&pos).expect("hit").mv, "2g2f");
    }

    #[test]
    fn key_ignores_move_number() {
        let a = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
        let sfen = a.to_sfen();
        let head = &sfen[..sfen.rfind(' ').expect("space")];
        let b = Position::from_sfen(&format!("{head} 99")).expect("ply99");
        assert_eq!(book_key(&a), book_key(&b));
    }

    #[test]
    fn miss_returns_none() {
        let pos = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
        let path = write_tmp("himawari_book_test2.db", "#YANEURAOU-DB2016 1.00\n");
        let book = Book::load(&path).expect("load");
        assert!(book.probe(&pos).is_none());
    }
}

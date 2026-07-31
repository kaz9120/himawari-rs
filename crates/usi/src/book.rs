//! 定跡の読み込みと参照（ADR-0063、ADR-0109のG10）。
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
//!
//! 引き方は参照実装の `BookMoveSelector::probe_impl`（book.cpp:1543-1820）に
//! 揃えた。合法手だけを残し、深さと評価値で足切りし、残った候補から選ぶ。

use std::collections::HashMap;

use himawari_core::{Color, Move, Position};

/// 定跡の1手（book.h:52-70）。db形式の1行に対応する。
#[derive(Clone)]
pub struct BookEntry {
    pub mv: String,
    /// 相手の予想応手。db形式の `none` は空文字で持つ（book.h:57）
    pub ponder: String,
    pub value: i32,
    pub depth: i32,
    pub move_count: u64,
}

/// 定跡を引いた結果（book.h:26-45の `ProbeResult`）。
pub struct BookHit {
    pub mv: Move,
    /// 予想応手。なければ `Move::NONE`
    pub ponder: Move,
    pub value: i32,
    pub depth: i32,
}

/// 定跡の引き方を決める値（book.cpp:1308-1392のadd_options）。
/// 既定値は参照実装のBOOK_OPTIONS=V1のものをそのまま使う。
#[derive(Clone, Debug)]
pub struct BookParams {
    /// エンジン側の定跡を使うか（book.cpp:1307）
    pub own_book: bool,
    /// 定跡を使う手数の上限。参照実装の `BookMoves`（book.cpp:1314）に
    /// あたるが、本エンジンは既存の `BookDepth`（既定24。ADR-0063）を使う
    pub book_moves: u16,
    /// 定跡を無視する確率[%]（book.cpp:1317、1550-1552）
    pub ignore_rate: u32,
    /// 最善手との評価値の差がこの範囲なら候補に残す（book.cpp:1364）
    pub eval_diff: i32,
    /// 先手番のときの評価値の下限（book.cpp:1365）
    pub eval_black_limit: i32,
    /// 後手番のときの評価値の下限（book.cpp:1366）
    pub eval_white_limit: i32,
    /// 最善手の深さがこれ未満なら定跡を使わない。0で無視（book.cpp:1373）
    pub depth_limit: i32,
    /// 盤面を反転した局面でも定跡を引く（book.cpp:1391）
    pub flipped: bool,
    /// 候補から一様ランダムに選ぶ（本エンジン独自）。
    /// 参照実装は常にランダムに選ぶ（book.cpp:1764）。本エンジンは
    /// SPRTの再現性を保つため既定を決定的にし、切り替えを選べるようにした
    pub randomize: bool,
}

impl Default for BookParams {
    fn default() -> Self {
        BookParams {
            own_book: true,
            book_moves: 24,
            ignore_rate: 0,
            eval_diff: 30,
            eval_black_limit: 0,
            eval_white_limit: -140,
            depth_limit: 16,
            flipped: true,
            randomize: false,
        }
    }
}

/// xorshift64*（misc.hのPRNG相当）。ランダム選択が有効なときだけ回す。
pub struct Prng(u64);

impl Prng {
    fn new() -> Prng {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x1234_5678_9abc_def0);
        Prng(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(2685821657736338717)
    }

    /// 0以上n未満を返す。nが0なら0。
    fn rand(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
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

/// 大文字と小文字を入れ替える（先後の入れ替え）。
fn swap_case(c: char) -> char {
    if c.is_ascii_uppercase() {
        c.to_ascii_lowercase()
    } else {
        c.to_ascii_uppercase()
    }
}

/// sfenのキーを180度回転させ、先後を入れ替える
/// （position.cppの `Position::sfen_to_flipped_sfen`）。
/// 盤面・手番・手駒の3つを反転させる。形が壊れていればNoneを返す。
fn flip_key(key: &str) -> Option<String> {
    let mut f = key.split_whitespace();
    let board = f.next()?;
    let stm = f.next()?;
    let hands = f.next()?;
    if f.next().is_some() {
        return None;
    }

    // 盤面を81マスへ展開する。段は1段目から、各段は9筋から並ぶ。
    // 180度回転は全マスの並びの反転と等しい
    let mut cells: Vec<String> = Vec::with_capacity(81);
    for rank in board.split('/') {
        let mut n = 0;
        let mut it = rank.chars();
        while let Some(c) = it.next() {
            if let Some(d) = c.to_digit(10) {
                for _ in 0..d {
                    cells.push(String::new());
                }
                n += d as usize;
            } else if c == '+' {
                cells.push(format!("+{}", swap_case(it.next()?)));
                n += 1;
            } else {
                cells.push(swap_case(c).to_string());
                n += 1;
            }
        }
        if n != 9 {
            return None;
        }
    }
    if cells.len() != 81 {
        return None;
    }
    cells.reverse();

    let mut out = String::with_capacity(key.len() + 8);
    for r in 0..9 {
        if r > 0 {
            out.push('/');
        }
        let mut empty = 0;
        for c in &cells[r * 9..r * 9 + 9] {
            if c.is_empty() {
                empty += 1;
                continue;
            }
            if empty > 0 {
                out.push_str(&empty.to_string());
                empty = 0;
            }
            out.push_str(c);
        }
        if empty > 0 {
            out.push_str(&empty.to_string());
        }
    }

    out.push(' ');
    out.push(match stm {
        "b" => 'w',
        "w" => 'b',
        _ => return None,
    });
    out.push(' ');
    out.push_str(&flip_hands(hands)?);
    Some(out)
}

/// 手駒の先後を入れ替える。順序は `Position::to_sfen` の慣例に戻す。
fn flip_hands(hands: &str) -> Option<String> {
    if hands == "-" {
        return Some("-".to_string());
    }
    let mut count: HashMap<char, u32> = HashMap::new();
    let mut n = 0u32;
    let mut seen = false;
    for c in hands.chars() {
        if let Some(d) = c.to_digit(10) {
            n = n * 10 + d;
            seen = true;
        } else if c.is_ascii_alphabetic() {
            *count.entry(swap_case(c)).or_insert(0) += if seen { n } else { 1 };
            n = 0;
            seen = false;
        } else {
            return None;
        }
    }
    if seen {
        return None;
    }
    let mut out = String::new();
    for c in "RBGSNLPrbgsnlp".chars() {
        let Some(&k) = count.get(&c) else { continue };
        if k == 0 {
            continue;
        }
        if k > 1 {
            out.push_str(&k.to_string());
        }
        out.push(c);
    }
    if out.is_empty() { None } else { Some(out) }
}

/// USI表記の指し手を180度回転させる（types.hの `flip_move`）。
/// 空文字はそのまま返す（予想応手なしを表す）。
fn flip_usi_move(m: &str) -> Option<String> {
    if m.is_empty() {
        return Some(String::new());
    }
    let b = m.as_bytes();
    let flip_sq = |s: &[u8]| -> Option<String> {
        let file = s.first().copied()?;
        let rank = s.get(1).copied()?;
        if !file.is_ascii_digit() || !(b'a'..=b'i').contains(&rank) {
            return None;
        }
        let nf = b'0' + 10 - (file - b'0');
        let nr = b'a' + 8 - (rank - b'a');
        String::from_utf8(vec![nf, nr]).ok()
    };
    // 駒打ち（例 P*5e）は駒種をそのままに、打つ場所だけ反転させる
    if b.len() == 4 && b[1] == b'*' {
        return Some(format!("{}*{}", m.chars().next()?, flip_sq(&b[2..])?));
    }
    if b.len() != 4 && b.len() != 5 {
        return None;
    }
    let mut out = format!("{}{}", flip_sq(&b[0..2])?, flip_sq(&b[2..4])?);
    if b.len() == 5 {
        if b[4] != b'+' {
            return None;
        }
        out.push('+');
    }
    Some(out)
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
            let ponder = match f.get(1) {
                Some(&"none") | Some(&"resign") | None => String::new(),
                Some(s) => (*s).to_string(),
            };
            entries.entry(k.clone()).or_default().push(BookEntry {
                mv: f[0].to_string(),
                ponder,
                value: f.get(2).and_then(|s| s.parse().ok()).unwrap_or(0),
                depth: f.get(3).and_then(|s| s.parse().ok()).unwrap_or(0),
                move_count: f.get(4).and_then(|s| s.parse().ok()).unwrap_or(0),
            });
        }
        // 出現回数の降順、同数なら評価値の降順に並べる（book.h:87-89、
        // book.cpp:75-88）。先頭が足切りの基準になる
        for v in entries.values_mut() {
            v.sort_by(|a, b| b.move_count.cmp(&a.move_count).then(b.value.cmp(&a.value)));
        }
        Ok(Book { entries })
    }

    pub fn positions(&self) -> usize {
        self.entries.len()
    }

    /// 局面に対応する候補手を返す（book.cpp:1219-1280の `find_in_books`）。
    /// 直接ヒットしなければ、盤面を反転した局面で引き直し、
    /// 見つかれば指し手を反転して返す。
    fn find(&self, pos: &Position, p: &BookParams) -> Option<Vec<BookEntry>> {
        let key = book_key(pos);
        if let Some(v) = self.entries.get(&key) {
            return Some(v.clone());
        }
        if !p.flipped {
            return None;
        }
        let v = self.entries.get(&flip_key(&key)?)?;
        let mut out = Vec::with_capacity(v.len());
        for e in v {
            out.push(BookEntry {
                mv: flip_usi_move(&e.mv)?,
                ponder: flip_usi_move(&e.ponder)?,
                value: e.value,
                depth: e.depth,
                move_count: e.move_count,
            });
        }
        Some(out)
    }

    /// 定跡を引く（book.cpp:1543-1820の `probe_impl`）。
    /// `notes` には足切りの理由など、GUIへ流す文言を積む。
    pub fn probe(
        &self,
        pos: &Position,
        p: &BookParams,
        prng: &mut Prng,
        notes: &mut Vec<String>,
    ) -> Option<BookHit> {
        if !p.own_book {
            return None;
        }
        // 一定確率で定跡を無視する（book.cpp:1550-1552）
        if u64::from(p.ignore_rate) > prng.rand(100) {
            return None;
        }
        // 定跡を用いる手数（book.cpp:1554-1557）
        if pos.game_ply() > p.book_moves {
            return None;
        }
        let mut list = self.find(pos, p)?;
        if list.is_empty() {
            return None;
        }
        // 非合法手の排除（book.cpp:1580-1614）。move_from_usiは合法手と
        // 照合するので、これだけで足りる
        let mut moves: Vec<(BookEntry, Move)> = Vec::with_capacity(list.len());
        for e in list.drain(..) {
            if let Some(m) = pos.move_from_usi(&e.mv) {
                moves.push((e, m));
            } else {
                notes.push(format!(
                    "Error! : Illegal Move In Book DB : move = {} , sfen = {}",
                    e.mv,
                    pos.to_sfen()
                ));
            }
        }
        if moves.is_empty() {
            return None;
        }
        // 深さの足切り（book.cpp:1721-1729）。先頭手の深さだけを見る。
        // 同じ評価値で深さ違いの手があるとき、片側だけ消えるのを避ける
        if p.depth_limit != 0 && moves[0].0.depth < p.depth_limit {
            notes.push("BookDepthLimit is lower than the depth of this node.".to_string());
            return None;
        }
        // 評価値の足切り（book.cpp:1736-1747）
        let value_limit1 = moves[0].0.value - p.eval_diff;
        let value_limit2 = if pos.side_to_move() == Color::Black {
            p.eval_black_limit
        } else {
            p.eval_white_limit
        };
        let value_limit = value_limit1.max(value_limit2);
        let n = moves.len();
        moves.retain(|(e, _)| e.value >= value_limit);
        if n != moves.len() {
            notes.push(format!(
                "BookEvalDiff = {} , limit = {value_limit2} , {n} moves to {} moves.",
                p.eval_diff,
                moves.len()
            ));
        }
        if moves.is_empty() {
            return None;
        }
        // 候補から1手選ぶ（book.cpp:1764）。参照実装は一様ランダムだが、
        // 既定は決定的に先頭（出現回数と評価値で最上位）を採る
        let idx = if p.randomize {
            prng.rand(moves.len() as u64) as usize
        } else {
            0
        };
        let (entry, mv) = &moves[idx];
        let mut ponder = Move::NONE;
        let mut child = pos.clone();
        child.do_move(*mv);
        if !entry.ponder.is_empty() {
            ponder = child.move_from_usi(&entry.ponder).unwrap_or(Move::NONE);
        }
        // 予想応手が無いなら、1手進めた局面の定跡から先頭手を借りる
        // （book.cpp:1805-1820）
        if ponder == Move::NONE
            && let Some(next) = self.find(&child, p)
            && let Some(e) = next.first()
        {
            ponder = child.move_from_usi(&e.mv).unwrap_or(Move::NONE);
        }
        Some(BookHit {
            mv: *mv,
            ponder,
            value: entry.value,
            depth: entry.depth,
        })
    }
}

/// 定跡の設定と状態（ADR-0063、ADR-0109のG10）。
/// 探索器には渡さずUSI層で閉じる。
pub struct BookOptions {
    pub file: String,
    pub params: BookParams,
    pub book: Option<Book>,
    prng: Prng,
}

impl Default for BookOptions {
    fn default() -> Self {
        BookOptions {
            file: String::new(),
            params: BookParams::default(),
            book: None,
            prng: Prng::new(),
        }
    }
}

impl BookOptions {
    /// 定跡を引く。ヒットしなければNone。`notes` にGUIへ流す文言が積まれる。
    pub fn probe(&mut self, pos: &Position, notes: &mut Vec<String>) -> Option<BookHit> {
        let book = self.book.as_ref()?;
        book.probe(pos, &self.params, &mut self.prng, notes)
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

    fn probe(book: &Book, pos: &Position, p: &BookParams) -> Option<BookHit> {
        let mut prng = Prng(1);
        let mut notes = Vec::new();
        book.probe(pos, p, &mut prng, &mut notes)
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
        let hit = probe(&book, &pos, &BookParams::default()).expect("hit");
        assert_eq!(hit.mv.to_usi(), "2g2f");
        assert_eq!(hit.ponder.to_usi(), "8c8d");
        assert_eq!(hit.value, 55);
        assert_eq!(hit.depth, 24);
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
        assert!(probe(&book, &pos, &BookParams::default()).is_none());
    }

    #[test]
    fn depth_limit_rejects_shallow_entry() {
        let pos = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
        let body = format!(
            "#YANEURAOU-DB2016 1.00\nsfen {}\n7g7f 3c3d 42 8 1\n",
            book_key(&pos)
        );
        let path = write_tmp("himawari_book_test3.db", &body);
        let book = Book::load(&path).expect("load");
        // 既定のBookDepthLimit=16を下回るので引かない
        assert!(probe(&book, &pos, &BookParams::default()).is_none());
        let p = BookParams {
            depth_limit: 0,
            ..BookParams::default()
        };
        assert!(probe(&book, &pos, &p).is_some());
    }

    #[test]
    fn eval_limit_rejects_low_value() {
        let pos = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
        // 先手番の下限は0。負の評価値しかなければ引かない
        let body = format!(
            "#YANEURAOU-DB2016 1.00\nsfen {}\n7g7f 3c3d -50 24 1\n",
            book_key(&pos)
        );
        let path = write_tmp("himawari_book_test4.db", &body);
        let book = Book::load(&path).expect("load");
        assert!(probe(&book, &pos, &BookParams::default()).is_none());
        let p = BookParams {
            eval_black_limit: -99999,
            ..BookParams::default()
        };
        assert!(probe(&book, &pos, &p).is_some());
    }

    #[test]
    fn eval_diff_keeps_only_close_moves() {
        let pos = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
        let body = format!(
            "#YANEURAOU-DB2016 1.00\nsfen {}\n2g2f 8c8d 55 24 1\n7g7f 3c3d 20 24 1\n",
            book_key(&pos)
        );
        let path = write_tmp("himawari_book_test5.db", &body);
        let book = Book::load(&path).expect("load");
        // 差35はBookEvalDiff=30を超えるので7g7fは落ちる。
        // ランダム選択を有効にしても残るのは2g2fだけ
        let p = BookParams {
            randomize: true,
            ..BookParams::default()
        };
        for _ in 0..20 {
            assert_eq!(probe(&book, &pos, &p).expect("hit").mv.to_usi(), "2g2f");
        }
    }

    #[test]
    fn ponder_is_filled_from_child_position() {
        let pos = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
        let mut child = pos.clone();
        child.do_move(pos.move_from_usi("7g7f").expect("legal"));
        let body = format!(
            "#YANEURAOU-DB2016 1.00\nsfen {}\n7g7f none 42 24 1\nsfen {}\n3c3d 2g2f 10 24 1\n",
            book_key(&pos),
            book_key(&child)
        );
        let path = write_tmp("himawari_book_test6.db", &body);
        let book = Book::load(&path).expect("load");
        let hit = probe(&book, &pos, &BookParams::default()).expect("hit");
        assert_eq!(hit.mv.to_usi(), "7g7f");
        assert_eq!(hit.ponder.to_usi(), "3c3d");
    }

    #[test]
    fn flipped_book_hits_mirrored_position() {
        // 先手が7六歩を指した局面を、後手番の同型局面として登録しておく
        let start = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
        let mut after = start.clone();
        after.do_move(start.move_from_usi("7g7f").expect("legal"));
        let body = format!(
            "#YANEURAOU-DB2016 1.00\nsfen {}\n7g7f 3c3d 42 24 1\n",
            book_key(&start)
        );
        let path = write_tmp("himawari_book_test7.db", &body);
        let book = Book::load(&path).expect("load");
        // 平手初期局面の後手番は先手番と反転同型。3c3dが返るはず
        let flipped =
            Position::from_sfen("lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL w - 1")
                .expect("sfen");
        let hit = probe(&book, &flipped, &BookParams::default()).expect("hit");
        assert_eq!(hit.mv.to_usi(), "3c3d");
        assert_eq!(hit.ponder.to_usi(), "7g7f");
        let p = BookParams {
            flipped: false,
            ..BookParams::default()
        };
        assert!(probe(&book, &flipped, &p).is_none());
    }

    #[test]
    fn flip_key_is_an_involution() {
        for key in [
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b -",
            "l6nl/5+P1gk/2np1S3/p1p4Pp/3P2Sp1/1PPb2P1P/P5GS1/R8/LN4bKL w RGgsn5p",
        ] {
            let f = flip_key(key).expect("flip");
            assert_eq!(flip_key(&f).expect("flip back"), key);
        }
        // 平手初期局面は反転しても盤面が同じで手番だけ変わる
        assert_eq!(
            flip_key("lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b -")
                .expect("flip"),
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL w -"
        );
    }

    #[test]
    fn flip_move_round_trips() {
        for m in ["7g7f", "3c3d", "8h2b+", "P*5e", "1a9i"] {
            let f = flip_usi_move(m).expect("flip");
            assert_eq!(flip_usi_move(&f).expect("flip back"), m);
        }
        assert_eq!(flip_usi_move("7g7f").expect("flip"), "3c3d");
        assert_eq!(flip_usi_move("P*5e").expect("flip"), "P*5e");
        assert_eq!(flip_usi_move("").expect("flip"), "");
    }
}

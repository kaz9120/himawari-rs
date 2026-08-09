//! CSA棋譜の最小パーサ（ADR-0152）。
//!
//! floodgateの棋譜を再生するのに要る範囲だけを読む。対局者名（N+/N-）、
//! 指し手（`+7776FU`）、消費時間（T行）、終局（`%TORYO` 等）である。
//! 変化・分岐や任意の開始局面は扱わない。floodgateの棋譜は平手初期局面
//! から始まる本譜1本なので、それ以外は未対応として弾く。
//!
//! 指し手のCSA表記は移動先の駒種しか持たない。どの駒がどこから動いたかは
//! from/toで決まるが、成りかどうかは「fromの駒」と「駒種」の差でしか
//! 分からない。自前で盤面を持つと二重管理になるので、合法手生成と
//! 突き合わせて解決する（[`resolve_move`]）。

use himawari_core::{
    Color, File, Move, MoveList, PieceType, Position, Rank, Square, generate_legal,
};

/// 平手初期局面のP行。floodgateの棋譜はすべてこの形で始まる。
const HIRATE_ROWS: [&str; 9] = [
    "P1-KY-KE-GI-KI-OU-KI-GI-KE-KY",
    "P2 * -HI *  *  *  *  * -KA *",
    "P3-FU-FU-FU-FU-FU-FU-FU-FU-FU",
    "P4 *  *  *  *  *  *  *  *  *",
    "P5 *  *  *  *  *  *  *  *  *",
    "P6 *  *  *  *  *  *  *  *  *",
    "P7+FU+FU+FU+FU+FU+FU+FU+FU+FU",
    "P8 * +KA *  *  *  *  * +HI *",
    "P9+KY+KE+GI+KI+OU+KI+GI+KE+KY",
];

/// 1手ぶんの記録。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsaMove {
    pub color: Color,
    /// 打つ手はNone（CSAでは `00`）。
    pub from: Option<Square>,
    pub to: Square,
    /// 移動後の駒種（成る手は成駒）。
    pub piece: PieceType,
    /// T行の消費時間[秒]。T行がなければNone。
    pub time_s: Option<u64>,
    /// 元の表記（`+7776FU`）。レポートへそのまま出す。
    pub text: String,
}

/// 1局ぶんの記録。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CsaGame {
    pub black: String,
    pub white: String,
    pub event: Option<String>,
    pub start_time: Option<String>,
    /// `'Increment:` の値[秒]。持ち時間の推定に使う。
    pub increment_s: Option<u64>,
    pub moves: Vec<CsaMove>,
    /// 終局の表記（`%TORYO`・`%TIME_UP` など）。棋譜が途中で切れていればNone。
    pub end: Option<String>,
    /// `'summary:` 行の中身。勝敗の判定に使う。
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CsaError {
    /// 平手初期局面で始まっていない。
    NotHirate,
    /// 指し手の表記を読めない。
    BadMove(String),
    /// 指し手が1つもない。
    NoMoves,
}

impl std::fmt::Display for CsaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CsaError::NotHirate => write!(f, "平手初期局面で始まっていない（未対応）"),
            CsaError::BadMove(s) => write!(f, "指し手を読めない: {s}"),
            CsaError::NoMoves => write!(f, "指し手が1つもない"),
        }
    }
}

impl std::error::Error for CsaError {}

impl CsaGame {
    /// 勝者。引き分け・不明はNone。`'summary:` 行の `win:<名前>` から採る。
    pub fn winner(&self) -> Option<Color> {
        let summary = self.summary.as_deref()?;
        let name = summary
            .split(':')
            .find_map(|f| f.strip_suffix(" win").map(str::trim))?;
        if name == self.black {
            Some(Color::Black)
        } else if name == self.white {
            Some(Color::White)
        } else {
            None
        }
    }

    /// 対局者名。
    pub fn player(&self, c: Color) -> &str {
        match c {
            Color::Black => &self.black,
            Color::White => &self.white,
        }
    }

    /// 名前に `needle` を含む側を返す。両方含めば先手を返す。
    pub fn side_of(&self, needle: &str) -> Option<Color> {
        if self.black.contains(needle) {
            Some(Color::Black)
        } else if self.white.contains(needle) {
            Some(Color::White)
        } else {
            None
        }
    }
}

/// CSA表記の駒種（`FU`・`TO` など）。
fn piece_type(code: &str) -> Option<PieceType> {
    Some(match code {
        "FU" => PieceType::PAWN,
        "KY" => PieceType::LANCE,
        "KE" => PieceType::KNIGHT,
        "GI" => PieceType::SILVER,
        "KI" => PieceType::GOLD,
        "KA" => PieceType::BISHOP,
        "HI" => PieceType::ROOK,
        "OU" => PieceType::KING,
        "TO" => PieceType::PRO_PAWN,
        "NY" => PieceType::PRO_LANCE,
        "NK" => PieceType::PRO_KNIGHT,
        "NG" => PieceType::PRO_SILVER,
        "UM" => PieceType::HORSE,
        "RY" => PieceType::DRAGON,
        _ => return None,
    })
}

/// CSA表記のマス（`77`）。`00` は打つ手なのでNone扱いにするため、
/// ここでは筋・段が1〜9のときだけSomeを返す。
fn square(text: &str) -> Option<Square> {
    let b = text.as_bytes();
    if b.len() != 2 {
        return None;
    }
    let file = b[0].checked_sub(b'1')?;
    let rank = b[1].checked_sub(b'1')?;
    if file < 9 && rank < 9 {
        Some(Square::new(File(file), Rank(rank)))
    } else {
        None
    }
}

/// 指し手行（`+7776FU`）を読む。指し手行でなければNone。
fn parse_move_line(line: &str) -> Option<Result<CsaMove, CsaError>> {
    let bytes = line.as_bytes();
    if bytes.len() != 7 {
        return None;
    }
    let color = match bytes[0] {
        b'+' => Color::Black,
        b'-' => Color::White,
        _ => return None,
    };
    if !line[1..5].bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let bad = || Some(Err(CsaError::BadMove(line.to_string())));
    let Some(piece) = piece_type(&line[5..7]) else {
        return bad();
    };
    let Some(to) = square(&line[3..5]) else {
        return bad();
    };
    let from = if &line[1..3] == "00" {
        None
    } else {
        match square(&line[1..3]) {
            Some(sq) => Some(sq),
            None => return bad(),
        }
    };
    Some(Ok(CsaMove {
        color,
        from,
        to,
        piece,
        time_s: None,
        text: line.to_string(),
    }))
}

/// 消費時間行（`T12`）を読む。小数（`T1.5`）は切り捨てる。
fn parse_time_line(line: &str) -> Option<u64> {
    let value = line.strip_prefix('T')?;
    if let Ok(n) = value.parse::<u64>() {
        return Some(n);
    }
    value.parse::<f64>().ok().map(|v| v.max(0.0) as u64)
}

/// CSA棋譜を読む。
pub fn parse(text: &str) -> Result<CsaGame, CsaError> {
    let mut game = CsaGame::default();
    let mut rows: Vec<String> = Vec::new();
    let mut hirate_shorthand = false;

    for raw in text.lines() {
        let line = raw.trim_end_matches(['\r', '\n', ' ', '\t']);
        if line.is_empty() {
            continue;
        }
        if let Some(body) = line.strip_prefix('\'') {
            // コメント行。持ち時間と結果だけを拾う
            if let Some(v) = body.strip_prefix("Increment:") {
                game.increment_s = v.trim().parse().ok();
            } else if let Some(v) = body.strip_prefix("summary:") {
                game.summary = Some(v.trim().to_string());
            }
            continue;
        }
        if let Some(v) = line.strip_prefix("N+") {
            game.black = v.to_string();
        } else if let Some(v) = line.strip_prefix("N-") {
            game.white = v.to_string();
        } else if let Some(v) = line.strip_prefix("$EVENT:") {
            game.event = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("$START_TIME:") {
            game.start_time = Some(v.to_string());
        } else if line == "PI" {
            hirate_shorthand = true;
        } else if line.starts_with('P') {
            rows.push(line.to_string());
        } else if let Some(end) = line.strip_prefix('%') {
            game.end = Some(format!("%{end}"));
        } else if let Some(t) = parse_time_line(line) {
            if let Some(last) = game.moves.last_mut() {
                last.time_s = Some(t);
            }
        } else if let Some(parsed) = parse_move_line(line) {
            game.moves.push(parsed?);
        }
        // 手番行（"+" 単独）とその他の行は読み飛ばす
    }

    if !hirate_shorthand && rows != HIRATE_ROWS {
        return Err(CsaError::NotHirate);
    }
    if game.moves.is_empty() {
        return Err(CsaError::NoMoves);
    }
    Ok(game)
}

/// CSAの指し手を、その局面の合法手と突き合わせて解決する。
///
/// from・to・移動後の駒種が一致する合法手を選ぶ。この3つが揃えば
/// 合法手はひとつに定まる（成り・不成は駒種が違う）。
pub fn resolve_move(pos: &Position, m: &CsaMove) -> Option<Move> {
    if pos.side_to_move() != m.color {
        return None;
    }
    // allをtrueにする。実戦には不成（角不成・飛不成など）が現れるが、
    // 探索向けの生成はそれを省く。省いた側で照合すると棋譜が読めなくなる
    let mut list = MoveList::default();
    generate_legal(pos, true, &mut list);
    list.as_slice().iter().copied().find(|&mv| {
        if mv.to() != m.to {
            return false;
        }
        match m.from {
            None => mv.is_drop() && mv.drop_piece_type() == m.piece,
            Some(from) => {
                !mv.is_drop() && mv.from_sq() == from && mv.piece_after().piece_type() == m.piece
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use himawari_core::SFEN_STARTPOS;

    const SAMPLE: &str = "V2\n\
        N+Himawari_6fd5a66\n\
        N-Opponent\n\
        'Increment:10\n\
        $EVENT:wdoor+floodgate-300-10F+Himawari_6fd5a66+Opponent+20260809103000\n\
        $START_TIME:2026/08/09 10:30:00\n\
        P1-KY-KE-GI-KI-OU-KI-GI-KE-KY\n\
        P2 * -HI *  *  *  *  * -KA * \n\
        P3-FU-FU-FU-FU-FU-FU-FU-FU-FU\n\
        P4 *  *  *  *  *  *  *  *  * \n\
        P5 *  *  *  *  *  *  *  *  * \n\
        P6 *  *  *  *  *  *  *  *  * \n\
        P7+FU+FU+FU+FU+FU+FU+FU+FU+FU\n\
        P8 * +KA *  *  *  *  * +HI * \n\
        P9+KY+KE+GI+KI+OU+KI+GI+KE+KY\n\
        +\n\
        +2726FU\n\
        T3\n\
        '** 120 -3334FU\n\
        -3334FU\n\
        T12\n\
        +7776FU\n\
        T5\n\
        -2233KA\n\
        T4\n\
        +8833UM\n\
        T1\n\
        %TORYO\n\
        'summary:toryo:Himawari_6fd5a66 win:Opponent lose\n\
        '$END_TIME:2026/08/09 10:41:00\n";

    #[test]
    fn parses_header_moves_and_result() {
        let game = parse(SAMPLE).expect("パースできる");
        assert_eq!(game.black, "Himawari_6fd5a66");
        assert_eq!(game.white, "Opponent");
        assert_eq!(game.increment_s, Some(10));
        assert_eq!(game.start_time.as_deref(), Some("2026/08/09 10:30:00"));
        assert_eq!(game.end.as_deref(), Some("%TORYO"));
        assert_eq!(game.moves.len(), 5);
        assert_eq!(game.winner(), Some(Color::Black));
        assert_eq!(game.side_of("Himawari"), Some(Color::Black));
        assert_eq!(game.side_of("いない"), None);
    }

    /// 消費時間は直前の指し手に付く。コメント行は読み飛ばす。
    #[test]
    fn attaches_time_to_preceding_move() {
        let game = parse(SAMPLE).expect("パースできる");
        let times: Vec<_> = game.moves.iter().map(|m| m.time_s).collect();
        assert_eq!(times, vec![Some(3), Some(12), Some(5), Some(4), Some(1)]);
    }

    #[test]
    fn parses_move_fields() {
        let game = parse(SAMPLE).expect("パースできる");
        let first = &game.moves[0];
        assert_eq!(first.color, Color::Black);
        assert_eq!(first.from, Some(Square::new(File(1), Rank(6))));
        assert_eq!(first.to, Square::new(File(1), Rank(5)));
        assert_eq!(first.piece, PieceType::PAWN);
        assert_eq!(first.text, "+2726FU");
    }

    /// 打つ手はfromがNoneになる。
    #[test]
    fn parses_drop() {
        let m = parse_move_line("+0057KI")
            .expect("指し手行")
            .expect("読める");
        assert_eq!(m.from, None);
        assert_eq!(m.to, Square::new(File(4), Rank(6)));
        assert_eq!(m.piece, PieceType::GOLD);
    }

    #[test]
    fn rejects_non_move_lines() {
        assert!(parse_move_line("T12").is_none());
        assert!(parse_move_line("+").is_none());
        assert!(parse_move_line("%TORYO").is_none());
        assert!(parse_move_line("+2726XX").expect("7文字").is_err());
    }

    /// 平手以外の開始局面は未対応として弾く。誤って再生すると
    /// レポートの局面がすべてずれる。
    #[test]
    fn rejects_non_hirate_start() {
        let handicap = SAMPLE.replace("P2 * -HI *  *  *  *  * -KA * \n", "P2 * -HI * \n");
        assert_eq!(parse(&handicap), Err(CsaError::NotHirate));
    }

    #[test]
    fn accepts_hirate_shorthand() {
        let mut lines: Vec<&str> = SAMPLE.lines().filter(|l| !l.starts_with('P')).collect();
        lines.insert(6, "PI");
        let game = parse(&lines.join("\n")).expect("PIは平手");
        assert_eq!(game.moves.len(), 5);
    }

    #[test]
    fn rejects_empty_game() {
        let header = SAMPLE
            .lines()
            .take_while(|l| !l.starts_with("+2726FU"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(parse(&header), Err(CsaError::NoMoves));
    }

    /// 合法手生成と突き合わせて解決する。成りは駒種で判別する。
    #[test]
    fn resolves_moves_against_legal_moves() {
        let game = parse(SAMPLE).expect("パースできる");
        let mut pos = Position::from_sfen(SFEN_STARTPOS).expect("初期局面");
        let usi: Vec<String> = game
            .moves
            .iter()
            .map(|m| {
                let mv = resolve_move(&pos, m).expect("合法手に一致する");
                pos.do_move(mv);
                mv.to_usi()
            })
            .collect();
        assert_eq!(usi, vec!["2g2f", "3c3d", "7g7f", "2b3c", "8h3c+"]);
    }

    /// 手番が合わない指し手は解決しない。棋譜が壊れていれば止める。
    #[test]
    fn rejects_move_of_wrong_side() {
        let pos = Position::from_sfen(SFEN_STARTPOS).expect("初期局面");
        let m = parse_move_line("-3334FU")
            .expect("指し手行")
            .expect("読める");
        assert_eq!(resolve_move(&pos, &m), None);
    }
}

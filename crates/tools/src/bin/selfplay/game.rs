//! 1局の実行と終局判定（ADR-0027）。
//!
//! 終局判定はマネージャ側で行う。合法手なし=詰み、同一局面4回=千日手
//! （連続王手は反則）、手数上限=引き分け、resign=投了、非合法手=即負け。
//! 時間切れはマネージャの計測で判定する。

use std::collections::HashMap;
use std::time::Duration;

use himawari_core::{Color, MoveList, Position, Repetition, generate_legal};

use crate::engine::UsiEngine;

pub enum TimeControl {
    Fischer { base_ms: u64, inc_ms: u64 },
    Nodes(u64),
}

pub struct GameConfig {
    pub tc: TimeControl,
    /// 手数上限（ply数）。到達で引き分け。
    pub max_moves: usize,
    /// スコアによる早期終局 (閾値cp, 連続ply数)。Noneで無効。
    pub adjudicate: Option<(i32, u32)>,
}

pub struct GameRecord {
    /// Noneは引き分け。
    pub winner: Option<Color>,
    pub reason: &'static str,
    pub moves: Vec<String>,
}

impl GameRecord {
    fn end(winner: Option<Color>, reason: &'static str, moves: Vec<String>) -> Self {
        GameRecord {
            winner,
            reason,
            moves,
        }
    }
}

pub fn play_game(
    black: &mut UsiEngine,
    white: &mut UsiEngine,
    opening: &str,
    cfg: &GameConfig,
) -> Result<GameRecord, String> {
    black.new_game()?;
    white.new_game()?;
    let mut pos =
        Position::from_sfen(opening).map_err(|e| format!("開始局面が不正 ({e:?}): {opening}"))?;
    let mut counts: HashMap<u64, u32> = HashMap::new();
    counts.insert(pos.key(), 1);
    let mut moves: Vec<String> = Vec::new();
    let mut clock: [i64; 2] = match cfg.tc {
        TimeControl::Fischer { base_ms, .. } => [base_ms as i64; 2],
        TimeControl::Nodes(_) => [0; 2],
    };
    // スコア打ち切り用: 各エンジンの直近評価値（先手視点）と連続ply数
    let mut last_view: [Option<i32>; 2] = [None, None];
    let mut streak: [u32; 2] = [0, 0];

    loop {
        if moves.len() >= cfg.max_moves {
            return Ok(GameRecord::end(None, "maxmoves", moves));
        }
        let mut list = MoveList::default();
        generate_legal(&pos, true, &mut list);
        let stm = pos.side_to_move();
        if list.is_empty() {
            return Ok(GameRecord::end(Some(stm.flip()), "mate", moves));
        }

        let pos_cmd = if moves.is_empty() {
            format!("position sfen {opening}")
        } else {
            format!("position sfen {opening} moves {}", moves.join(" "))
        };
        let (go_cmd, timeout) = match cfg.tc {
            TimeControl::Fischer { inc_ms, .. } => (
                format!(
                    "go btime {} wtime {} binc {inc_ms} winc {inc_ms}",
                    clock[0].max(0),
                    clock[1].max(0)
                ),
                // 残り時間を使い切った上で加算・猶予を上乗せした上限
                Duration::from_millis(clock[stm.index()].max(0) as u64 + inc_ms + 10_000),
            ),
            TimeControl::Nodes(n) => (format!("go nodes {n}"), Duration::from_secs(600)),
        };
        let engine: &mut UsiEngine = if stm == Color::Black { black } else { white };
        let r = engine.think(&pos_cmd, &go_cmd, timeout)?;

        if let TimeControl::Fischer { inc_ms, .. } = cfg.tc {
            let c = &mut clock[stm.index()];
            *c -= r.elapsed_ms as i64;
            if *c < 0 {
                return Ok(GameRecord::end(Some(stm.flip()), "timeloss", moves));
            }
            *c += inc_ms as i64;
        }
        if r.bestmove == "resign" {
            return Ok(GameRecord::end(Some(stm.flip()), "resign", moves));
        }
        if r.bestmove == "win" {
            // 宣言勝ち（ADR-0030）。27点法で検証し、不当な宣言は反則負け
            return if pos.can_declare_win() {
                Ok(GameRecord::end(Some(stm), "declaration", moves))
            } else {
                Ok(GameRecord::end(Some(stm.flip()), "declaration_invalid", moves))
            };
        }
        let Some(m) = pos.move_from_usi(&r.bestmove) else {
            return Ok(GameRecord::end(Some(stm.flip()), "illegal", moves));
        };
        pos.do_move(m);
        moves.push(r.bestmove.clone());

        let c = counts.entry(pos.key()).or_insert(0);
        *c += 1;
        if *c >= 4 {
            match pos.repetition_state() {
                Repetition::Draw => return Ok(GameRecord::end(None, "repetition", moves)),
                // Win/Loseは手番側から見た連続王手の千日手の判定
                Repetition::Win => {
                    return Ok(GameRecord::end(
                        Some(pos.side_to_move()),
                        "repetition_foul",
                        moves,
                    ));
                }
                Repetition::Lose => {
                    return Ok(GameRecord::end(
                        Some(pos.side_to_move().flip()),
                        "repetition_foul",
                        moves,
                    ));
                }
                _ => {}
            }
        }

        if let Some((threshold, need)) = cfg.adjudicate {
            if let Some(sc) = r.score_cp {
                last_view[stm.index()] = Some(if stm == Color::Black { sc } else { -sc });
            }
            // 両エンジンの直近評価が同符号で閾値を超えている間だけ延ばす
            streak = match (last_view[0], last_view[1]) {
                (Some(a), Some(b)) if a >= threshold && b >= threshold => [streak[0] + 1, 0],
                (Some(a), Some(b)) if a <= -threshold && b <= -threshold => [0, streak[1] + 1],
                _ => [0, 0],
            };
            if streak[0] >= need {
                return Ok(GameRecord::end(Some(Color::Black), "adjudication", moves));
            }
            if streak[1] >= need {
                return Ok(GameRecord::end(Some(Color::White), "adjudication", moves));
            }
        }
    }
}

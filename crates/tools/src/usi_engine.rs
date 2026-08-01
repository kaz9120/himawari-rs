//! USIエンジンのサブプロセスクライアント（ADR-0027・ADR-0122）。
//!
//! stdout読み取りは専用スレッドでチャネルに流し、待ち受けはすべて
//! タイムアウト付きで行う。ハングは実装バグとして即エラーにする。
//! プロセスの後始末は `Drop` が担うので、呼び出し側にtrap相当の処理は要らない。

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub struct UsiEngine {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<String>,
    path: String,
}

/// info行から拾う値。表へそのまま出す文字列と、比較に使う数値を持つ。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InfoLine {
    pub depth: Option<u32>,
    pub nodes: Option<u64>,
    pub time_ms: Option<u64>,
    /// scoreをUSIの表記のまま持つ（"cp 123" / "mate 5"）。
    pub score: Option<String>,
    /// scoreを手番視点の数値へ写像した値（mateは±30000近傍）。
    pub score_cp: Option<i32>,
}

/// info行を解析する。info行でなければNone。
pub fn parse_info(line: &str) -> Option<InfoLine> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.first() != Some(&"info") {
        return None;
    }
    let mut info = InfoLine::default();
    for (i, token) in tokens.iter().enumerate() {
        match *token {
            "depth" => info.depth = tokens.get(i + 1).and_then(|v| v.parse().ok()),
            "nodes" => info.nodes = tokens.get(i + 1).and_then(|v| v.parse().ok()),
            "time" => info.time_ms = tokens.get(i + 1).and_then(|v| v.parse().ok()),
            "score" => match (tokens.get(i + 1), tokens.get(i + 2)) {
                (Some(&"cp"), Some(v)) => {
                    info.score = Some(format!("cp {v}"));
                    info.score_cp = v.parse().ok();
                }
                (Some(&"mate"), Some(v)) => {
                    info.score = Some(format!("mate {v}"));
                    if let Ok(n) = v.parse::<i32>() {
                        info.score_cp = Some(if n >= 0 { 30000 - n } else { -30000 - n });
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    Some(info)
}

pub struct ThinkResult {
    pub bestmove: String,
    /// bestmoveに付随する予測応手（ponderヒント。ADR-0033）。
    pub ponder: Option<String>,
    /// 最後にinfoで報告された評価値（手番視点、mateは±30000近傍へ写像）。
    pub score_cp: Option<i32>,
    pub elapsed_ms: u64,
    /// nodesを持つ最後のinfo行。NPS計測が読む（ADR-0122）。
    pub last_info: InfoLine,
    /// `go_depth` で指定した深さの、nodesを持つ最初のinfo行。
    /// 機能検証がこの行のノード数を比べる（ADR-0074）。指定深さへ
    /// 到達せずに探索が終わればNone。
    pub target_depth_info: Option<InfoLine>,
}

impl UsiEngine {
    pub fn launch(path: &str, options: &[(String, String)]) -> Result<UsiEngine, String> {
        Self::launch_with_args(path, &[], options)
    }

    /// 起動コマンドに引数を渡して立ち上げる。`profile` が
    /// `samply record ... <engine>` の形で包んで起動するために要る
    /// （ADR-0122）。エンジンのstdin/stdoutは包んだプロセスを素通りする。
    pub fn launch_with_args(
        program: &str,
        args: &[&str],
        options: &[(String, String)],
    ) -> Result<UsiEngine, String> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("{program} を起動できません: {e}"))?;
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(l) = line else { break };
                if tx.send(l).is_err() {
                    return;
                }
            }
        });
        let mut eng = UsiEngine {
            child,
            stdin,
            rx,
            path: program.to_string(),
        };
        eng.send("usi")?;
        eng.wait_for("usiok", Duration::from_secs(10))?;
        for (name, value) in options {
            eng.send(&format!("setoption name {name} value {value}"))?;
        }
        eng.send("isready")?;
        eng.wait_for("readyok", Duration::from_secs(60))?;
        Ok(eng)
    }

    pub fn send(&mut self, cmd: &str) -> Result<(), String> {
        writeln!(self.stdin, "{cmd}")
            .and_then(|_| self.stdin.flush())
            .map_err(|e| format!("{}: 送信失敗 ({e})", self.path))
    }

    fn wait_for(&mut self, token: &str, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(format!("{}: {token} 待ちでタイムアウト", self.path));
            }
            match self.rx.recv_timeout(deadline - now) {
                Ok(l) if l.trim() == token => return Ok(()),
                Ok(_) => {}
                Err(_) => {
                    return Err(format!(
                        "{}: {token} 待ちでタイムアウト（またはプロセス終了）",
                        self.path
                    ));
                }
            }
        }
    }

    /// 対局間の区切り。TT等の状態をリセットし、完了を同期する。
    pub fn new_game(&mut self) -> Result<(), String> {
        self.send("usinewgame")?;
        self.send("isready")?;
        self.wait_for("readyok", Duration::from_secs(60))
    }

    pub fn think(
        &mut self,
        position_cmd: &str,
        go_cmd: &str,
        timeout: Duration,
    ) -> Result<ThinkResult, String> {
        self.send(position_cmd)?;
        let start = Instant::now();
        self.send(go_cmd)?;
        self.collect(start, timeout, None)
    }

    /// 固定深さで読ませる。指定深さのinfo行を `target_depth_info` へ入れて
    /// 返す。機能検証とNPS計測が使う（ADR-0074・ADR-0081）。
    pub fn go_depth(
        &mut self,
        position_cmd: &str,
        depth: u32,
        timeout: Duration,
    ) -> Result<ThinkResult, String> {
        self.send(position_cmd)?;
        let start = Instant::now();
        self.send(&format!("go depth {depth}"))?;
        self.collect(start, timeout, Some(depth))
    }

    /// bestmove行が来るまで待つ。経過時間はstart起点で測る
    /// （ponderhit後の計測に使う。ADR-0033）。
    pub fn wait_bestmove(
        &mut self,
        start: Instant,
        timeout: Duration,
    ) -> Result<ThinkResult, String> {
        self.collect(start, timeout, None)
    }

    /// bestmoveまでのinfo行を集める。target_depthを与えると、その深さの
    /// 最初のinfo行も拾う。
    fn collect(
        &mut self,
        start: Instant,
        timeout: Duration,
        target_depth: Option<u32>,
    ) -> Result<ThinkResult, String> {
        let deadline = start + timeout;
        let mut score_cp = None;
        let mut last_info = InfoLine::default();
        let mut target_depth_info = None;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(format!("{}: bestmove待ちでタイムアウト", self.path));
            }
            let line = self.rx.recv_timeout(deadline - now).map_err(|_| {
                format!(
                    "{}: bestmove待ちでタイムアウト（またはプロセス終了）",
                    self.path
                )
            })?;
            let elapsed_ms = start.elapsed().as_millis() as u64;
            let tokens: Vec<&str> = line.split_whitespace().collect();
            match tokens.first() {
                Some(&"bestmove") => {
                    let mv = tokens
                        .get(1)
                        .ok_or_else(|| format!("{}: 不正なbestmove行: {line}", self.path))?;
                    let ponder = if tokens.get(2) == Some(&"ponder") {
                        tokens.get(3).map(|s| s.to_string())
                    } else {
                        None
                    };
                    return Ok(ThinkResult {
                        bestmove: mv.to_string(),
                        ponder,
                        score_cp,
                        elapsed_ms,
                        last_info,
                        target_depth_info,
                    });
                }
                Some(&"info") => {
                    let Some(info) = parse_info(&line) else {
                        continue;
                    };
                    if let Some(cp) = info.score_cp {
                        score_cp = Some(cp);
                    }
                    // currmove行はnodesを持たない。集計から外す
                    if info.nodes.is_none() {
                        continue;
                    }
                    if target_depth_info.is_none() && info.depth == target_depth {
                        target_depth_info = Some(info.clone());
                    }
                    last_info = info;
                }
                _ => {}
            }
        }
    }

    pub fn quit(self) {
        // 対局中のエンジンは即座に終わる。応じなければDropで回収する
        self.quit_within(Duration::from_secs(1));
    }

    /// quitを送り、プロセスの終了をtimeoutまで待つ。samplyで包んだときは
    /// エンジンの終了後にプロファイルを書き出すため、長めに待つ。
    pub fn quit_within(mut self, timeout: Duration) {
        let _ = self.send("quit");
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // quitに応じないプロセスは強制終了（Dropで回収）
    }
}

impl Drop for UsiEngine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_info_extracts_numbers_and_score() {
        let line = "info depth 13 seldepth 21 score cp -42 nodes 123456 nps 800000 time 154 \
                    hashfull 12 pv 7g7f 3c3d";
        let info = parse_info(line).expect("info行");
        assert_eq!(info.depth, Some(13));
        assert_eq!(info.nodes, Some(123_456));
        assert_eq!(info.time_ms, Some(154));
        assert_eq!(info.score.as_deref(), Some("cp -42"));
        assert_eq!(info.score_cp, Some(-42));
    }

    /// mateの写像はselfplayの早期終局判定と揃える（ADR-0027）。
    #[test]
    fn parse_info_maps_mate_to_score() {
        let win = parse_info("info depth 9 score mate 5 nodes 100 time 1 pv 1a1b").expect("info行");
        assert_eq!(win.score.as_deref(), Some("mate 5"));
        assert_eq!(win.score_cp, Some(29995));
        let lose =
            parse_info("info depth 9 score mate -3 nodes 100 time 1 pv 1a1b").expect("info行");
        assert_eq!(lose.score.as_deref(), Some("mate -3"));
        assert_eq!(lose.score_cp, Some(-29997));
    }

    /// lowerbound付きでも値は拾う（ADR-0091）。currmove行はnodesを持たない。
    #[test]
    fn parse_info_handles_bound_and_currmove() {
        let bound = parse_info("info depth 13 score cp 30 lowerbound nodes 50 time 2 pv 7g7f")
            .expect("info行");
        assert_eq!(bound.score.as_deref(), Some("cp 30"));
        assert_eq!(bound.nodes, Some(50));
        let currmove = parse_info("info depth 13 currmove 7g7f").expect("info行");
        assert_eq!(currmove.depth, Some(13));
        assert_eq!(currmove.nodes, None);
        assert_eq!(currmove.score, None);
    }

    #[test]
    fn parse_info_rejects_other_lines() {
        assert!(parse_info("bestmove 7g7f ponder 3c3d").is_none());
        assert!(parse_info("").is_none());
    }
}

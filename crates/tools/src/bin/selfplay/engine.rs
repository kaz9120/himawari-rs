//! USIエンジンのサブプロセスクライアント（ADR-0027）。
//!
//! stdout読み取りは専用スレッドでチャネルに流し、待ち受けはすべて
//! タイムアウト付きで行う。ハングは実装バグとして即エラーにする。

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

pub struct ThinkResult {
    pub bestmove: String,
    /// 最後にinfoで報告された評価値（手番視点、mateは±30000近傍へ写像）。
    pub score_cp: Option<i32>,
    pub elapsed_ms: u64,
}

impl UsiEngine {
    pub fn launch(path: &str, options: &[(String, String)]) -> Result<UsiEngine, String> {
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("{path} を起動できません: {e}"))?;
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
            path: path.to_string(),
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

    fn send(&mut self, cmd: &str) -> Result<(), String> {
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
        let deadline = start + timeout;
        let mut score_cp = None;
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
                    return Ok(ThinkResult {
                        bestmove: mv.to_string(),
                        score_cp,
                        elapsed_ms,
                    });
                }
                Some(&"info") => {
                    if let Some(i) = tokens.iter().position(|&t| t == "score") {
                        match (tokens.get(i + 1), tokens.get(i + 2)) {
                            (Some(&"cp"), Some(v)) => score_cp = v.parse().ok(),
                            (Some(&"mate"), Some(v)) => {
                                if let Ok(n) = v.parse::<i32>() {
                                    score_cp = Some(if n >= 0 { 30000 - n } else { -30000 - n });
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    pub fn quit(mut self) {
        let _ = self.send("quit");
        for _ in 0..50 {
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

//! エンジン本体のバイナリが持つ、USI層以外の部品。
//!
//! `usi_engine`（USIクライアント）と `game`（対局ループ）は、もとは
//! 開発ツール側にあった。動作環境の最適なスレッド数を1バイナリで測る
//! `threadtune` モードが自分自身を子プロセスとして起動して
//! 対局させるため、エンジン側へ移した。開発ツール（`himawari-tools`）は
//! ここを参照する。依存はstdと `himawari-core` だけで、配布物の
//! 依存ゼロは保つ。

pub mod game;
pub mod threadtune;
pub mod usi_engine;

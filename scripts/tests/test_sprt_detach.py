"""scripts/sprt-detach.py の引数処理と冪等性を検証する。

実際にSPRTを起動する経路は副作用が大きいので触らない。起動しないで返る
分岐（引数エラー・判定済み）だけを確かめる。
"""

from conftest import load_module

sprt_detach = load_module("sprt_detach", "sprt-detach.py")


def test_help_returns_zero(capsys):
    assert sprt_detach.main(["--help"]) == 0
    assert "使い方" in capsys.readouterr().out


def test_too_few_arguments_returns_two(capsys):
    assert sprt_detach.main(["base", "cand"]) == 2
    assert "使い方" in capsys.readouterr().err


def test_malformed_env_assignment_returns_two(capsys):
    code = sprt_detach.main(["base", "cand", "name", "SPRT_ELO0"])
    assert code == 2
    assert "KEY=VALUE" in capsys.readouterr().err


def test_existing_result_short_circuits(tmp_path, monkeypatch, capsys):
    """判定済みなら起動せず、結果をそのまま返す（ADR-0175の冪等性）。"""
    repo = tmp_path
    (repo / "data" / "sprt").mkdir(parents=True)
    (repo / "scripts").mkdir()
    (repo / "scripts" / "sprt-run.sh").write_text("#!/bin/sh\n", encoding="utf-8")
    (repo / "data" / "sprt" / "x.result").write_text(
        "name=x\ndecision=H1\nelo=+11.2\n", encoding="utf-8"
    )

    # __file__ を差し替えてリポジトリルートの解決先を tmp_path にする
    monkeypatch.setattr(sprt_detach, "__file__", str(repo / "scripts" / "sprt-detach.py"))

    def fail(*args, **kwargs):
        raise AssertionError("判定済みなのに起動しようとした")

    monkeypatch.setattr(sprt_detach.subprocess, "Popen", fail)

    assert sprt_detach.main(["base", "cand", "x"]) == 0
    out = capsys.readouterr().out
    assert "判定済み" in out
    assert "decision=H1" in out

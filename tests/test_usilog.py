"""USIログの1局ごとの構造化を検証する。入力は #545 の抜粋から組み直したログ。"""

import json
from pathlib import Path

from hmwr import cli, proc
from hmwr.tools import usilog

LOG = Path(__file__).parent / "golden" / "usilog-545.log"


def _games():
    return usilog.split_games(usilog.read_lines([LOG]))


def test_the_log_splits_into_two_games_ended_by_gameover():
    games = _games()
    assert [g["ended_by"] for g in games] == ["gameover", "gameover"]
    assert [g["result"] for g in games] == ["lose", "lose"]
    assert all(g["schema"] == usilog.SCHEMA for g in games)
    assert games[0]["engine"]["name"] == "Himawari 0.40.0"


def test_path_options_are_dropped_and_others_kept():
    options = _games()[0]["engine"]["options"]
    assert options == {"Threads": "6", "USI_Ponder": "true"}


def test_the_hang_of_545_is_a_move_without_bestmove():
    hang = _games()[0]["moves"][-1]
    assert hang["outcome"] == "no_bestmove"
    assert hang["go"] == {
        "ponder": True,
        "btime": 29000,
        "wtime": 59000,
        "binc": 10000,
        "winc": 10000,
    }
    assert hang["ponderhit_ms"] == 6663
    assert hang["last_currmove"] == {"t": 10273, "depth": 11, "move": "2d1d"}
    plan = hang["time_plan"]
    assert (plan["t"], plan["maximum"], plan["elapsed"], plan["stop"]) == (
        28546,
        27870,
        28546,
        True,
    )
    assert hang["ply"] == 7 and hang["side"] == "b"


def test_think_time_counts_from_ponderhit_and_ponder_misses_are_separated():
    moves = _games()[0]["moves"]
    assert [m["outcome"] for m in moves] == [
        "bestmove",
        "bestmove",
        "ponder_miss",
        "bestmove",
        "no_bestmove",
    ]
    assert moves[0]["think_ms"] == 4988
    assert moves[1]["ponderhit_ms"] == 3000 and moves[1]["think_ms"] == 1008
    assert moves[1]["iterations"][-1]["score"] == {"mate": 9}
    assert moves[2]["think_ms"] is None and moves[2]["bestmove"] == "2f2e"
    assert moves[3]["iterations"][0]["bound"] == "lowerbound"
    assert moves[3]["last_pv_change_depth"] == 14


def test_the_second_game_never_answers_go():
    game = _games()[1]
    assert game["summary"]["moves"] == 0
    assert game["summary"]["no_bestmove"] == 1
    assert game["moves"][0]["side"] == "w"


def test_summary_averages_only_played_moves():
    s = _games()[0]["summary"]
    assert s["moves"] == 3 and s["ponder_hits"] == 1 and s["ponder_misses"] == 1
    assert s["avg_depth"] == round((14 + 15 + 14) / 3, 2)
    assert s["avg_nps"] == (2000000 + 4000000 + 3000000) * 1000 // (2400 + 4000 + 5000)


def test_a_log_starting_mid_game_and_waits_are_recorded():
    rows = [
        (100, "<", "position startpos moves 7g7f"),
        (101, "<", "go btime 1000 wtime 1000 byoyomi 0"),
        (400, ">", "info string waiting for threads 2,5 elapsed 200"),
        (500, ">", "info string path = C:\\\\u{5c06}\\u{68cb}"),
        (600, "<", "usinewgame"),
        (650, ">", "info string waiting for previous search: thread 3 elapsed 1000"),
    ]
    first, second = usilog.split_games(rows)
    assert first["started_by"] == "log_start" and first["ended_by"] == "usinewgame"
    move = first["moves"][0]
    assert move["waits"] == [
        {"t": 299, "kind": "wait_threads", "threads": [2, 5], "elapsed": 200}
    ]
    assert move["notes"][0]["text"] == "path = C:\\将棋"
    assert second["events"] == [
        {"t": 50, "kind": "wait_previous_search", "threads": [3], "elapsed": 1000}
    ]
    assert second["ended_by"] == "log_end"


def test_daily_files_are_joined_by_time(tmp_path):
    lines = LOG.read_text(encoding="utf-8").splitlines()
    half = len(lines) // 2
    (tmp_path / "usi-2.log").write_text("\n".join(lines[half:]) + "\n", encoding="utf-8")
    (tmp_path / "usi-1.log").write_text("\n".join(lines[:half]) + "\n", encoding="utf-8")
    joined = usilog.read_lines([tmp_path / "usi-2.log", tmp_path / "usi-1.log"])
    assert joined == usilog.read_lines([LOG])


def test_the_command_writes_one_json_per_game(tmp_path, capsys):
    out = tmp_path / "games"
    assert cli.main(["kifu", "usilog", str(LOG), "--out", str(out)]) == proc.OK
    files = sorted(out.glob("*.json"))
    assert len(files) == 2
    game = json.loads(files[0].read_text(encoding="utf-8"))
    assert game["schema"] == usilog.SCHEMA
    assert f"| {files[0].stem} | gameover | lose | 3 |" in capsys.readouterr().out


def test_dry_run_writes_nothing(tmp_path, capsys):
    out = tmp_path / "games"
    assert cli.main(["--dry-run", "kifu", "usilog", str(LOG), "--out", str(out)]) == proc.OK
    assert not out.exists()
    assert "[dry-run] write" in capsys.readouterr().out

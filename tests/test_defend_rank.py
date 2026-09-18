"""攻撃的な受けの局面での順位の集計を検証する（ADR-0213）。

期待値は手で数えられる小さな表で置く。`psv defend` が書くTSVと同じ列を
並べ、群の分け方・平均・中央値・上位の割合が定義どおりかを見る。
"""

import pytest

from hmwr.tools import defend_rank as dr

HEADER = "\t".join(dr.COLUMNS)
# index, defend, moves, rank, value, best
ROWS = (
    (1, 1, 40, 1, 100, 100),
    (2, 1, 30, 5, 0, 120),
    (3, 1, 50, 9, -50, 150),
    (4, 0, 40, 1, 10, 10),
    (5, 0, 20, 3, 20, 60),
)


def write_tsv(tmp_path, rows=ROWS, header=HEADER):
    path = tmp_path / "defend.tsv"
    lines = [header] + ["\t".join(str(v) for v in row) for row in rows]
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return path


def test_rows_split_into_the_two_groups(tmp_path):
    stats = dr.aggregate(dr.read_rows(write_tsv(tmp_path)))
    assert stats["攻撃的な受け"]["n"] == 3
    assert stats["一般"]["n"] == 2


def test_rank_statistics_match_hand_counts(tmp_path):
    """該当群の順位は1・5・9で、平均5・中央値5・1位率は3分の1になる。"""
    stats = dr.aggregate(dr.read_rows(write_tsv(tmp_path)))
    s = stats["攻撃的な受け"]
    assert s["mean"] == pytest.approx(5.0)
    assert s["median"] == pytest.approx(5.0)
    assert s["top1"] == pytest.approx(100.0 / 3)
    # 3位以内は順位1の1件だけ
    assert s["topk"] == pytest.approx(100.0 / 3)
    # 平均損は 0・120・200 の平均
    assert s["loss"] == pytest.approx((0 + 120 + 200) / 3)


def test_median_averages_the_middle_pair(tmp_path):
    """件数が偶数なら中央2つの平均を返す。一般群の順位は1と3である。"""
    stats = dr.aggregate(dr.read_rows(write_tsv(tmp_path)))
    assert stats["一般"]["median"] == pytest.approx(2.0)
    assert stats["一般"]["top1"] == pytest.approx(50.0)
    assert stats["一般"]["topk"] == pytest.approx(100.0)


def test_report_shows_both_groups_and_their_gap(tmp_path):
    rows = dr.read_rows(write_tsv(tmp_path))
    text = "\n".join(dr.report(dr.aggregate(rows), rows, "x.tsv"))
    assert "攻撃的な受け" in text and "一般" in text
    # 該当5.00 − 一般2.00
    assert "+3.00" in text
    assert "該当率: 60.00%" in text


def test_a_group_without_positions_is_shown_as_such(tmp_path):
    rows = dr.read_rows(write_tsv(tmp_path, rows=ROWS[3:]))
    stats = dr.aggregate(rows)
    assert stats["攻撃的な受け"] is None
    text = "\n".join(dr.report(stats, rows, "x.tsv"))
    assert "（該当なし）" in text


def test_a_wrong_header_is_an_error(tmp_path):
    path = write_tsv(tmp_path, header="index\trank")
    with pytest.raises(ValueError):
        dr.read_rows(path)


def test_main_writes_the_table_to_the_log(tmp_path, capsys):
    path = write_tsv(tmp_path)
    log = tmp_path / "diag-defend-x.log"
    assert dr.main([str(path), "--log", str(log)]) == 0
    assert "攻撃的な受け" in capsys.readouterr().out
    assert "攻撃的な受け" in log.read_text(encoding="utf-8")

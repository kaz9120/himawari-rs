# 0244: 日本語文書のCIゲートをtextlintからyomiyasuへ替える

- Status: accepted
- Date: 2026-10-22
- 関連ADR: [0178](0178-textlint-gate.md), [0181](0181-agent-surface.md), [0209](0209-workflow-layers.md)

## Context

文書のCIゲートは[ADR-0178](0178-textlint-gate.md)のtextlintだった。一文の長さ・読点の数・
助詞の重複のように、文の形を見る規則が中心である。

文書の書き手はほぼエージェントで、残る癖はAIの文に出やすい語と書式である。
オーナーは手元の応答の検査に[yomiyasu](https://github.com/nanaism/yomiyasu)を使っており、
リポジトリの検査もこれへ替えるよう指示した（2026-10-22）。

yomiyasu v2.1.1を全259文書へ当てると、507件の指摘が出た。

| 規則 | 重さ | 件数 | 見るもの |
|---|---|---|---|
| slop_vocabulary | warn | 130 | AIの文に出やすい語（土台・台帳・ゲート・解像度） |
| negative_parallelism | info | 129 | 「AではなくB」 |
| excess_bold | warn | 82 | 1,000字あたりの太字の数 |
| excess_list | warn | 48 | 箇条書きの行の割合 |
| trailing_colon | warn | 45 | 文末のコロン |
| bold_not_rendered | error | 29 | GitHubで太字にならない `**` |
| metaphor_verb | warn | 21 | 抽象的な比喩の動詞 |
| translated_term | warn | 10 | 英語の直訳（版） |
| ほか | | 13 | ダッシュの装飾、否定の対比の密度 |

bold_not_renderedの29件は、表示が実際に崩れている。textlintは見ていなかった。

## 選択肢と比較

### 案A: textlintを残し、yomiyasuを足す

文の形の規則も残る。ただし検査器が2つになり、Nodeの依存も残る。
指示は「替える」であり、併用ではない。

### 案B: yomiyasuへ替える

同梱したyomiyasuを `hmwr doc lint` から呼ぶ。標準ライブラリだけで動くので、
CIのNodeとnpmの依存が消える。全文書の検査は1.4秒で終わる。

文の形の規則はyomiyasuに無く、失う。一文100字・読点3個まで・助詞の重複・
である調とです・ます調の混在・漢字の7字以上の連続である。

## Decision

案Bを採る。オーナーの指示による。

yomiyasuは pip で配られていないので、`hmwr/vendor/yomiyasu/` に同梱する。
`yomiyasu_lint.py` と、それが読む `markdown_visibility.py` の2ファイルである。
上流のv2.1.1のタグと同一であることを確かめた。ruffの対象からは外す。上げるときは
2ファイルと `VERSION` を差し替える。

yomiyasuは指摘を抑える仕組みを持たないので、`hmwr/doclint.py` で3つ足す。

1. 文書全体の割合を見るexcess_listとexcess_boldはゲートにしない。ADRのひな形が
   箇条書きで書く形で、割合だけでは良し悪しが決まらない
2. 本リポジトリの用語として文字どおりに使う語を通す。`hmwr data ledger` の台帳、
   SPRTゲート、効くの3語である。効くは「枝刈りが効く」の意味で使い、利きとも重なる
3. 直前の行に `<!-- yomiyasu-disable-next-line -->` を置くと、その行の指摘を外す。
   原文を変えられない引用に使う

ゲートにするのはwarnとerrorの指摘、それにinfoのbold_label_listである。
negative_parallelismはinfoのままにする。誤解を正す否定は残すべきで、機械が判別できない。

検査の対象は従来どおり `docs/`・ルートの `*.md`・`.claude/`・`research/` とする。
CHANGELOG.mdは外す。元のコミット件名を、Stopフックが同じ検査にかける。

## Consequences

CIのdocsジョブからNodeが消える。`npm ci` とtextlintで約45秒かかっていた検査が、約2秒になる。

一文の長さと読点の数は、機械が見なくなる。CLAUDE.mdの規約から外れた長い文が
入っても、CIは止めない。これが目立つようになったら、文の分割をyomiyasuの
解析に任せ、長さだけを `doclint.py` で数える案を検討する。

ゲートにした指摘の175件は、同じPRで直す。分けると、先に入れたほうが
textlintかyomiyasuのどちらかで落ちる。

`--fix` はなくなる。yomiyasuは自動で直さない。

## 見直すとき

- 上流がpipで配られるか、抑止の仕組みを持ったら、同梱と `doclint.py` の手当てを外す
- 通す語が5語を超えたら、規則ごと外すかをここへ追記して判断する

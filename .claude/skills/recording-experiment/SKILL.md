---
name: recording-experiment
description: 実験キューで走り終えた実験の結果を、ADRへ記録するPRを1本出す。hmwr exp reportの表を貼り、事前登録の解釈に照らして判断を書き、Statusと索引を更新する。実験のspecの名前とIssue番号を受け取って動く。キューがclaude -pで起動するほか、対話セッションで結果を記録するときも使う。
---

# 実験の結果を記録する

入力は2つ。specの名前（`experiments/<名前>.toml`）と、実験のIssue番号である。
出力はPR1本で、ADRへの結果の追記、Statusと索引の更新を含む。

**数値は手で書き写さない**。表は `hmwr exp report` の出力をそのまま貼る。
読みは事前登録に照らして書き、登録にない読みは「登録外」と明記する。
判断の大半は照合で済むように、ADRの側が事前登録している。

## 手順

1. 材料を集める

   ```
   hmwr exp report <名前>            # 対局・学習・データの表
   hmwr exp show <名前>              # 各ステップの完了時刻
   cat experiments/<名前>.toml       # adr = "NNNN" を読む
   ```

   ADRは `docs/adr/NNNN-*.md` にある。「仮説」「測定の設計」「解釈の
   事前登録」の節を読む。対局の数値は `data/sprt/<対局名>.result` が正で、
   表はそこから作られている

2. ブランチを切る。名前は `docs-adrNNNN-result` にする。専用のworktreeで
   走っているときは、いまのHEAD（origin/main）から切る

3. ADRへ「## 測定（日付）」の節を足す。置き場は「解釈の事前登録」の
   直後、「Consequences」の前にする。中身は次の順で書く

   - 走った条件のうち、事前登録から変わったものがあれば先に書く
     （再開が起きた、局数が違う、など）。無ければ「予定どおり走った」と書く
   - `hmwr exp report` の表をそのまま貼る。列を削ったり値を丸めたりしない
   - 読み。事前登録の表の行に当てはめ、どの行に当たるかを書く。閾値は
     ADRに書いてある値を使う。どの行にも当たらない結果は「登録外」と書き、
     解釈は1段落までにする。次の判断は対話セッションへ渡す
   - 事前登録にある「次の一手」を、その行のとおりに書く

4. 「## Decision」の節が仮説の採否を書く形なら、結果に沿って書く。Statusを
   更新する。仮説が支持されたらaccepted、棄却されたらrejected、登録外や
   判定に至らないならproposedのまま残し、理由を1行書く。Statusの括弧に
   要約を入れる（例: `accepted（量の20%増は効かず、別系列の混合が効いた）`）

5. 索引（`docs/adr/README.md`）の表のStatusを合わせる。ROADMAPは、ADRが
   「次の一手」で候補の整理を指示している場合だけ直す

6. 検査してPRを出す

   ```
   hmwr doc lint && hmwr doc check
   git add docs && git commit   # 型は docs、件名に実験名と結論を入れる
   git push -u origin docs-adrNNNN-result
   hmwr pr template chore > body.md   # 節を埋める
   hmwr pr create --kind chore --title "docs: …" --body-file body.md
   ```

   PRの本文には、表とどの行に当たったかを書き、Issue番号を `Refs #<番号>` で
   結ぶ。Issueは閉じない（マージした人が閉じる）

7. Issueへ、PRのURLと結論を1行ずつコメントする

8. 専用のworktreeで走っているときは、最後に `git switch --detach origin/main` で
   戻す。worktreeに変更を残さない

## しないこと

- コードを変えない。評価関数の差し替え（`hmwr/config.py`）もしない。採用の
  操作は対話セッションが行う
- マージしない。PRを出すところまでにする
- 事前登録の閾値を動かさない。「惜しい」を「効いた」に読み替えない
- 表の数値を変えない。桁の丸めもしない
- 局数を積み増す提案をしない。事前登録がそう決めている

## 例

[ADR-0210](../../../docs/adr/0210-teacher-mix-control.md)の「測定」と「Decision」の
節が、この手順で書いた形である。表、追試の読み、事前登録の表への当てはめ、
次の一手の順になっている。

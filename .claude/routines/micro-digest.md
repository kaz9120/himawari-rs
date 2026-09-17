# microの消化（毎晩）

[共通の約束](README.md)を先に読む。

1. 対象を1件選ぶ。無ければ何もせず終わる

   ```
   gh issue list --state open --label micro --label cloud --json number,title,createdAt \
     -q 'sort_by(.createdAt) | .[0]'
   ```

2. Issueを読み、CLAUDE.mdとmicro-improvementsスキルの「検証の型」に沿って
   直す。等価な書き換えは、等価性を機械で示す（テスト、`--dry-run` の出力の
   一致、バイト一致のどれか）
3. 読んでみて、クラウドでは扱えないと分かったら直さない。理由をコメントし、
   ラベルを `cloud` から `local` へ付け替えて終わる。当たるのは、計測が要る、
   `data/` が要る、棋力が変わる、設計判断が要る、のどれかである
4. 検査を通し、PRを出す。本文に `Closes #<番号>` を書く
5. CIが緑ならマージする。落ちたら1回だけ直す。それでも落ちたら、PRに状況を
   コメントして終わる

1回の実行で扱うのは1件だけにする。

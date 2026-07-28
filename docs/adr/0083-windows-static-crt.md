# 0083: WindowsバイナリをMSVCランタイム静的リンクで配布する

- Status: proposed
- Date: 2026-07-28
- 関連ADR: [0003](0003-toolchain.md), [0071](0071-release-please.md), [0081](0081-portability.md)

## Context

リリースしたWindows用バイナリが、実機で起動しなかった。
`VCRUNTIME140.dll` が見つからない旨のエラーが出る（2026-07-28、オーナー報告）。

Rustの `x86_64-pc-windows-msvc` ターゲットは、既定でMSVCのCランタイムへ
動的リンクする。実行には「Microsoft Visual C++ 再頒布可能パッケージ」が
必要になる。開発環境やVisual Studioの入ったマシンには同梱されているため
気づきにくいが、まっさらなWindowsには入っていない。

将棋エンジンは将棋所やShogiGUIから起動される。利用者に再頒布パッケージの
導入を求める構成は避けたい。ダウンロードしたzipを解凍して指定すれば動く
状態が望ましい。

## 選択肢と比較

### 案A: MSVCランタイムを静的リンクする

`RUSTFLAGS` に `-C target-feature=+crt-static` を足す。Rustが公式に
サポートする指定で、CRTがバイナリへ埋め込まれる。

DLLへの依存が消え、単体で動く。バイナリサイズは数百KB増える。

制約が1つある。静的CRTと動的CRTを混在させたプロセスでは、
メモリ確保と解放が別のヒープにまたがると壊れる。本エンジンは単体の
実行ファイルで、C ABIのDLLを読み込まないため該当しない。

### 案B: GNUツールチェイン（x86_64-pc-windows-gnu）へ切り替える

MinGWのランタイムを使う。こちらは既定で静的リンクに近い構成になる。

ただしターゲットを変えると、生成されるコードの性能特性が変わりうる。
本エンジンは計測が主目的であり、CPU別の最適化
（`-C target-cpu=x86-64-v3`）を効かせている。ツールチェインごと
替える理由としては弱い。

### 案C: 再頒布可能パッケージを案内する

READMEに導入手順を書く。実装コストはゼロ。

利用者に余計な手間を強いる。将棋GUIからエンジンを追加する場面で、
DLLが無いというエラーだけが出ても原因にたどり着けない。

## Decision

案Aを採る。`release.yml` のWindows向け `rustflags` に
`-C target-feature=+crt-static` を追加する。

```yaml
- os: windows-latest
  name: windows-x64-avx2
  rustflags: "-C target-cpu=x86-64-v3 -C target-feature=+crt-static"
```

`-C target-cpu` はCPUの命令セットを、`-C target-feature=+crt-static` は
リンク方法を指定する。両者は独立しており、併記できる。

LinuxとmacOSは変更しない。Linuxはglibcへ動的リンクするが、
実行環境に必ず存在する。静的リンク（musl等）へ移す必要は生じていない。

### 検証の限界

この修正の効果は、CI上では確かめられない。GitHub Actionsの
`windows-latest` にはMSVCランタイムが入っており、依存が残っていても
動いてしまう。`dumpbin /dependents` で依存の一覧は取れるが、
Developer Command Promptの初期化が要り、ワークフローが複雑になる。

まっさらなWindows機での実機確認をもって検証とする。
[ADR-0081](0081-portability.md)の移行先がその環境にあたる。

## Consequences

Windows用バイナリが単体で動くようになる。将棋GUIへ登録するとき、
zipを解凍して実行ファイルを指定するだけで済む。

バイナリサイズが数百KB増える。配布物として問題になる規模ではない。

コミットの型は `fix` とする。CI設定の変更だが、配布されるバイナリの
実体が変わるためである（[ADR-0071](0071-release-please.md)）。
`chore` にするとバージョンが上がらず、修正を含むリリースが作られない。

将来、C ABIのDLLを動的に読み込む機能を足す場合（外部の評価関数や
定跡ライブラリなど）は、静的CRTとの混在に注意が要る。
現時点でそうした計画はない。

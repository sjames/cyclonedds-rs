# CycloneDDS / iceoryx のビルド手順（vendor）

このディレクトリには、ワークスペースで利用する vendored 依存ライブラリをビルドするための `Makefile` が含まれています。

- `vendor/iceoryx` (v2.0.2)
- `vendor/cyclonedds` (releases/0.10.x)

## 1. サブモジュールを初期化する

リポジトリのルートで実行してください。

```bash
git submodule update --init --recursive
```

## 2. ビルド依存パッケージをインストールする（Ubuntu/Debian）

```bash
make setup
```

## 3. vendor 依存ライブラリをビルドする

```bash
make build
```

`make build` では次の処理を行います。

- `iceoryx` をビルドし、`vendor/iceoryx/install` にインストール
- `cyclonedds` を共有メモリ有効（`-DENABLE_SHM=YES`）でビルド
- インストール済み `iceoryx` を `CMAKE_PREFIX_PATH` に設定
- `cyclonedds` を `vendor/cyclonedds/install` にインストール

生成物（期待される成果物）:

- `vendor/iceoryx/install/bin/iox-roudi`
- `vendor/cyclonedds/install/lib/libddsc.so`

## 4. 環境変数を設定する

ビルド後に、以下の環境変数を設定してください（例: リポジトリルートの `.envrc`）。

```.envrc
export CYCLONEDDS_HOME=${PWD}/vendor/cyclonedds/install
export CYCLONEDDS_LIB_DIR=${CYCLONEDDS_HOME}/lib
export CYCLONEDDS_INCLUDE_DIR=${CYCLONEDDS_HOME}/include
```

## 5. クリーン

```bash
make clean
```

## 6. Debian パッケージを作成する

`cargo-deb` を使用して、runtime パッケージとヘッダを含む dev パッケージを作成できます。
`debian/Cargo.toml` は `debian/Cargo-template.toml` から自動生成されます。

```bash
cargo install cargo-deb
make -C debian deb
```

個別に作成する場合:

```bash
make -C debian runtime
make -C debian dev
```

`maintainer` / `copyright` を実行時に上書きする場合:

```bash
make -C debian deb \
	MAINTAINER="Your Team <dev@example.com>" \
	COPYRIGHT="2026, Your Team <dev@example.com>"
```

未指定時は `debian/Cargo.toml` の値が使われます。

生成された `.deb` はリポジトリルートの `deb/` に出力されます。

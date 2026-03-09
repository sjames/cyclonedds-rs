include ../../common.mk

# SHMを有効にする必要があるためiceoryxとまとめて1パッケージにする
PACKAGE_NAME?=cyclonedds-custom
PACKAGE_DEV_NAME?=${PACKAGE_NAME}-dev
PACKAGE_CONFIG_NAME?=${PACKAGE_NAME}-config
CRATE_NAME?=${PACKAGE_NAME}-deb
CRATE_VERSION?=0.10.5
HOMEPAGE?=https://github.com/uzuna/cyclonedds-pkg
CARGO_DEB_MANIFEST=${CURDIR}/Cargo.toml
CARGO_DEB_TEMPLATE=${CURDIR}/Cargo-template.toml
RENDER_SCRIPT=${CURDIR}/render_manifest.py
PYTHON ?= python3
MAINTAINER ?=Example Team <dev@example.com>
COPYRIGHT ?=2026, Example Team <dev@example.com>
# バイナリとヘッダーのパッケージバージョン
VERSION?=0.10.5
# configパッケージのバージョン
CONFIG_VERSION?=0.0.1

.PHONY: deb
deb: runtime dev config

.PHONY: generate-manifest
generate-manifest:
	${PYTHON} ${RENDER_SCRIPT} \
		--template ${CARGO_DEB_TEMPLATE} \
		--output ${CARGO_DEB_MANIFEST} \
		--crate-name "${CRATE_NAME}" \
		--crate-version "${CRATE_VERSION}" \
		--package-name "${PACKAGE_NAME}" \
		--package-dev-name "${PACKAGE_DEV_NAME}" \
		--package-config-name "${PACKAGE_CONFIG_NAME}" \
		--homepage "${HOMEPAGE}" \
		--maintainer "${MAINTAINER}" \
		--copyright "${COPYRIGHT}"

# バイナリビルド
${ICEORYX_BIN}:
	make -C ${VENDOR_DIR} ${ICEORYX_BIN}

# バイナリビルド
${CYCLONEDDS_LIB}:
	make -C ${VENDOR_DIR} ${CYCLONEDDS_LIB}

# runtime向けバイナリ
.PHONY: runtime
runtime: ${ICEORYX_BIN} ${CYCLONEDDS_LIB} generate-manifest
	mkdir -p ${DEB_DIR}
	cargo deb --manifest-path ${CARGO_DEB_MANIFEST} \
		--no-build \
		--no-strip \
		--deb-version ${VERSION} \
		--output ${DEB_DIR}

# cpp, rust, python pkgなどのビルドに必要なファイル
.PHONY: dev
dev: ${ICEORYX_BIN} ${CYCLONEDDS_LIB} generate-manifest
	mkdir -p ${DEB_DIR}
	cargo deb --manifest-path ${CARGO_DEB_MANIFEST} \
		--variant dev \
		--no-build \
		--no-strip \
		--deb-version ${VERSION} \
		--output ${DEB_DIR}

.PHONY: config
config: generate-manifest
	mkdir -p ${DEB_DIR}
	cargo deb --manifest-path ${CARGO_DEB_MANIFEST} \
		--variant config \
		--no-build \
		--no-strip \
		--deb-version ${CONFIG_VERSION} \
		--output ${DEB_DIR}


.PHONY: clean
clean:
	rm -rf ${DEB_DIR}/${PACKAGE_NAME}_*.deb
	rm -rf ${DEB_DIR}/${PACKAGE_DEV_NAME}_*.deb
	rm -rf ${DEB_DIR}/${PACKAGE_CONFIG_NAME}_*.deb
	rm -f ${CARGO_DEB_MANIFEST}
	rm -rf ${CURDIR}/target

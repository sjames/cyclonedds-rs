include ../common.mk

.PHONY: build
build: ${CYCLONEDDS_LIB} ${ICEORYX_BIN}

# vendor以下のビルドに必要なパッケージ
.PHONY: setup
setup:
	sudo apt install -y cmake build-essential git gcc g++ pkg-config \
		libacl1-dev libncurses5-dev

${ICEORYX_BIN}:
	cd iceoryx \
	&& cmake -Bbuild -Hiceoryx_meta -DCMAKE_INSTALL_PREFIX=install \
	&& cmake --build build --config RelWithDebInfo --target install

${CYCLONEDDS_LIB}: ${ICEORYX_BIN}
	cd cyclonedds \
	&& mkdir -p build install \
	&& cmake -Bbuild -DENABLE_SHM=YES \
		-DWERROR=OFF -DBUILD_IDLC=ON -DBUILD_DDSPERF=OFF -DBUILD_TESTING=OFF \
		-DBUILD_DDSPERF=OFF -DENABLE_TYPE_DISCOVERY=YES -DENABLE_TOPIC_DISCOVERY=YES \
		-DCMAKE_PREFIX_PATH=$(VENDOR_DIR)/iceoryx/install/ \
		-DCMAKE_INSTALL_PREFIX=install \
	&& cmake --build build --config RelWithDebInfo --target install

.PHONY: clean
clean:
	rm -rf iceoryx/build iceoryx/install cyclonedds/build cyclonedds/install

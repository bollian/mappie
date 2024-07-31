BUILD_IMAGE_NAME := "localhost/mappie"
TAG := "latest"
REMOTE_NAME := "ghcr.io/bollian/"
REMOTE_BUILD_IMAGE_NAME := REMOTE_NAME + BUILD_IMAGE_NAME

build: bt_ctrl_proxy robot operator_interface

build_img: build_img_aarch64 build_img_amd64

build_img_aarch64:
	buildah build --arch aarch64 --tag {{BUILD_IMAGE_NAME}}:{{TAG}}-aarch64 .

build_img_amd64:
	buildah build --arch amd64 --tag {{BUILD_IMAGE_NAME}}:{{TAG}}-amd64 .

build_env_aarch64:
	if podman container exists mappie_aarch64; then \
		podman start mappie_aarch64; \
	else \
		podman run --name mappie_aarch64 -d --security-opt label=disable --volume .:/var/tmp/build --arch aarch64 {{BUILD_IMAGE_NAME}}:{{TAG}}-aarch64 'sleep infinity'; \
	fi

build_env_amd64:
	if podman container exists mappie_amd64; then \
		podman start mappie_amd64; \
	else \
		podman run --name mappie_amd64 -d --security-opt label=disable --volume .:/var/tmp/build --arch amd64 {{BUILD_IMAGE_NAME}}:{{TAG}}-amd64 'sleep infinity'; \
	fi

alias bt := bt_ctrl_proxy
bt_ctrl_proxy: build_env_aarch64
	# bash is needed to setup $PATH for finding cargo
	podman exec mappie_aarch64 bash -lc 'cargo build --package bt-ctrl-proxy --target aarch64-unknown-linux-gnu'

robot: build_env_aarch64
	# bash is needed to setup $PATH for finding cargo
	podman exec mappie_aarch64 bash -lc 'cargo build --package robot --target aarch64-unknown-linux-gnu'

alias oi := operator_interface
operator_interface: build_env_amd64
	# bash is needed to setup $PATH for finding cargo
	podman exec mappie_amd64 bash -lc 'cargo build --package operator-interface --target x86_64-unknown-linux-gnu'

deploy:
	scp target/aarch64-unknown-linux-gnu/debug/{bt-ctrl-proxy,robot} mappie@rpi:~

clean:
	podman rm -f mappie_aarch64 mappie_amd64
	podman rmi {{BUILD_IMAGE_NAME}}:{{TAG}}-aarch64 {{BUILD_IMAGE_NAME}}:{{TAG}}-amd64
	cargo clean

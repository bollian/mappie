BUILD_IMAGE_NAME := "localhost/mappie"
TAG := "latest"
REMOTE_NAME := "ghcr.io/bollian/"
REMOTE_BUILD_IMAGE_NAME := REMOTE_NAME + BUILD_IMAGE_NAME
TARGET_DIR := "target/containerized"

profile := "release"

build: bt_ctrl_proxy robot operator_interface hardware_test

build_img: build_img_aarch64 build_img_x86_64

build_img_aarch64:
	buildah build --arch aarch64 --tag {{BUILD_IMAGE_NAME}}:{{TAG}}-aarch64 .

build_img_x86_64:
	buildah build --arch amd64 --tag {{BUILD_IMAGE_NAME}}:{{TAG}}-x86_64 .

build_env_aarch64:
	if podman container exists mappie_aarch64; then \
		podman start mappie_aarch64; \
	else \
		podman run --name mappie_aarch64 -d --security-opt label=disable --volume .:/var/tmp/build --arch aarch64 {{BUILD_IMAGE_NAME}}:{{TAG}}-aarch64 tail -F /dev/null; \
	fi

build_env_x86_64:
	if podman container exists mappie_x86_64; then \
		podman start mappie_x86_64; \
	else \
		podman run --name mappie_x86_64 -d --security-opt label=disable --volume .:/var/tmp/build --arch amd64 {{BUILD_IMAGE_NAME}}:{{TAG}}-x86_64 tail -F /dev/null; \
	fi

build_env: build_env_aarch64 build_env_x86_64

build_proj arch proj: build_env
	# bash is needed to setup $PATH for finding cargo
	podman exec mappie_{{arch}} bash -lc 'cargo build --package {{proj}} --target {{arch}}-unknown-linux-gnu --target-dir "{{TARGET_DIR}}" --profile={{profile}}'

alias bt := bt_ctrl_proxy
bt_ctrl_proxy: (build_proj "aarch64" "bt-ctrl-proxy")

robot: (build_proj "aarch64" "robot")

hardware_test: (build_proj "aarch64" "hardware-test")

alias oi := operator_interface
operator_interface: (build_proj "x86_64" "operator-interface")

deploy:
	scp target/containerized/aarch64-unknown-linux-gnu/{{profile}}/{robot,hardware-test} mappie@rpi:~

clean:
	podman rm -f mappie_aarch64 mappie_amd64
	podman rmi {{BUILD_IMAGE_NAME}}:{{TAG}}-aarch64 {{BUILD_IMAGE_NAME}}:{{TAG}}-amd64
	cargo clean

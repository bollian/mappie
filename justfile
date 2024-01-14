BASE_IMAGE_NAME := "mappie_base"
IMAGE_NAME := "mappie"
REMOTE_NAME := "ghcr.io/bollian/"
REMOTE_IMAGE_NAME := REMOTE_NAME + IMAGE_NAME
REMOTE_BASE_IMAGE_NAME := REMOTE_NAME + BASE_IMAGE_NAME
SAVE_FILE_NAME := IMAGE_NAME + ".tar.gz"

build:
	buildah build --arch aarch64 --file robot/Containerfile \
		--tag {{IMAGE_NAME}} --tag {{REMOTE_IMAGE_NAME}} robot

build_base:
	buildah build --arch aarch64 --file robot/Containerfile.base \
		--tag {{BASE_IMAGE_NAME}} --tag {{REMOTE_BASE_IMAGE_NAME}} robot

push: build
	podman push {{REMOTE_BASE_IMAGE_NAME}} {{REMOTE_IMAGE_NAME}}

save: build
	podman save -o {{SAVE_FILE_NAME}} {{IMAGE_NAME}}

IMAGE_NAME=mappie
REMOTE_IMAGE_NAME="ghcr.io/bollian/$IMAGE_NAME"
SAVE_FILE_NAME="mappie.tar.gz"

build:
	buildah build --arch aarch64 --tag $LOCAL_IMAGE_NAME --tag $REMOTE_IMAGE_NAME

push: build
	podman push $REMOTE_IMAGE_NAME

save: build
	podman save -o $SAVE_FILE_NAME $LOCAL_IMAGE_NAME

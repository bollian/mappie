FROM ubuntu:24.04

ADD --chmod=755 https://sh.rustup.rs /rustup.sh
RUN apt-get -y update \
	&& apt-get -y install curl gcc libudev-dev libdbus-1-dev \
	&& apt-get -y upgrade \
	&& /rustup.sh -y \
	&& rm /rustup.sh

WORKDIR /var/tmp/build

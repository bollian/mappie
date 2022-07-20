FROM docker.io/library/ros:humble

COPY . /opt/mappie
RUN apt-get -y update \
    && apt-get -y install python3-pip \
    && pip3 install /opt/mappie \
    && rm -rf /opt/mappie

ENTRYPOINT ["python3", "-m", "mappie"]

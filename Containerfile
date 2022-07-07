FROM docker.io/library/ros:humble

COPY . /opt/mappie
WORKDIR /opt/mappie
RUN apt-get -y update \
    && apt-get -y install python3-pip \
    && pip3 install -r requirements.txt

ENTRYPOINT ["python3", "/opt/mappie/src/mappie/main.py"]

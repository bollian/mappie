profile := "release"

alias b := build
build: robot hardware_test operator

lint: (robot "clippy") (hardware_test "clippy") (operator "clippy")

robot cmd="build": (_cmd_proj cmd "aarch64-unknown-linux-gnu" "robot")
hardware cmd="build": (_cmd_proj cmd "aarch64-unknown-linux-gnu" "hardware")

alias oix := operator
operator cmd="build": (_cmd_proj cmd "x86_64-unknown-linux-gnu" "oix")

alias htest := hardware_test
hardware_test cmd="build": (_cmd_proj cmd "aarch64-unknown-linux-gnu" "hardware-test")

alias ltest := local_test
local_test:
    cross test --target x86_64-unknown-linux-gnu -p robot -p oix -p hardware -p messages

alias d := deploy
deploy user="mappie" address="mappie" dir="~":
    scp hardware-config.json \
        robot-startup.sh \
        target/aarch64-unknown-linux-gnu/{{profile}}/{robot,hardware-test} \
        {{user}}@{{address}}:{{dir}}

_cmd_proj cmd arch proj:
    cross {{cmd}} --target {{arch}} --package {{proj}} --profile {{profile}}

clean:
    cross clean

doc:
    cross doc

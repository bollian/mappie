profile := "release"

alias b := build
build: robot operator_interface hardware_test oix

_build_proj arch proj:
	cross build --target {{arch}} --package {{proj}} --profile {{profile}}

robot: (_build_proj "aarch64-unknown-linux-gnu" "robot")
hardware: (_build_proj "aarch64-unknown-linux-gnu" "hardware")

alias htest := hardware_test
hardware_test: (_build_proj "aarch64-unknown-linux-gnu" "hardware-test")

alias oi := operator_interface
operator_interface: (_build_proj "x86_64-unknown-linux-gnu" "operator-interface")

oix: (_build_proj "x86_64-unknown-linux-gnu" "oix")

alias ltest := local_test
local_test:
    cross test --target x86_64-unknown-linux-gnu -p robot -p oix -p hardware -p messages

alias d := deploy
deploy user="mappie" address="mappie" dir="~":
	scp hardware-config.json \
		robot-startup.sh \
		target/aarch64-unknown-linux-gnu/{{profile}}/{robot,hardware-test} \
		{{user}}@{{address}}:{{dir}}

clean:
	cross clean

doc:
	cross doc

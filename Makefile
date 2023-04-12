EXECUTABLE_NAME := wait-service

all: ./target/release/$(EXECUTABLE_NAME)

./target/release/$(EXECUTABLE_NAME): $(shell find . -type f -iname '*.rs' -o -name 'Cargo.toml' | grep -v ./target | sed 's/ /\\ /g')
	cargo build --release --features json
	
install:
	$(MAKE)
	sudo cp ./target/release/$(EXECUTABLE_NAME) /usr/local/bin/$(EXECUTABLE_NAME)
	sudo chown root: /usr/local/bin/$(EXECUTABLE_NAME)
	sudo chmod 0755 /usr/local/bin/$(EXECUTABLE_NAME)

uninstall:
	sudo rm /usr/local/bin/$(EXECUTABLE_NAME)

test:
	cargo test --verbose

clean:
	cargo clean

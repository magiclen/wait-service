all: ./target/release/wait-service

./target/release/wait-service: $(shell find . -type f -iname '*.rs' -o -name 'Cargo.toml' | sed 's/ /\\ /g')
	cargo build --release --features json
	strip ./target/release/wait-service
	
install:
	$(MAKE)
	sudo cp ./target/release/wait-service /usr/local/bin/wait-service
	sudo chown root: /usr/local/bin/wait-service
	sudo chmod 0755 /usr/local/bin/wait-service

uninstall:
	sudo rm /usr/local/bin/wait-service

test:
	cargo test --verbose

clean:
	cargo clean

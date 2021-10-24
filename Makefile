all: ./target/x86_64-unknown-linux-musl/release/wait-service

./target/x86_64-unknown-linux-musl/release/wait-service: $(shell find . -type f -iname '*.rs' -o -name 'Cargo.toml' | sed 's/ /\\ /g')
	cargo build --release --target x86_64-unknown-linux-musl
	strip ./target/x86_64-unknown-linux-musl/release/wait-service
	
install:
	$(MAKE)
	sudo cp ./target/x86_64-unknown-linux-musl/release/wait-service /usr/local/bin/wait-service
	sudo chown root: /usr/local/bin/wait-service
	sudo chmod 0755 /usr/local/bin/wait-service

uninstall:
	sudo rm /usr/local/bin/wait-service

test:
	cargo test --verbose

clean:
	cargo clean

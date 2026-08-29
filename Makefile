KERNEL_DEV   = target/riscv64gc-unknown-none-elf/debug/osmium
KERNEL_REL   = target/riscv64gc-unknown-none-elf/release/osmium
QEMU         = qemu-system-riscv64
QEMU_FLAGS   = -machine virt -cpu rv64 -m 128M -smp 1 -nographic -bios default

.PHONY: build release run run-release test gdb objdump clean

build:
	cargo build

release:
	cargo build --release

# Boot the kernel with the serial console on stdio. Ctrl-A X exits QEMU.
run: build
	$(QEMU) $(QEMU_FLAGS) -kernel $(KERNEL_DEV)

run-release: release
	$(QEMU) $(QEMU_FLAGS) -kernel $(KERNEL_REL)

# Boot headless, capture the console, and verify the boot self-tests passed.
# The kernel powers QEMU off via the sifive_test device when tests finish.
test: build
	./scripts/boot-test.sh $(QEMU) "$(QEMU_FLAGS)" $(KERNEL_DEV)

# `make gdb` in one terminal, `riscv64-elf-gdb $(KERNEL_DEV)` + `target remote :1234` in another.
gdb: build
	$(QEMU) $(QEMU_FLAGS) -kernel $(KERNEL_DEV) -s -S

objdump: build
	rust-objdump -d --source $(KERNEL_DEV) | less

clean:
	cargo clean

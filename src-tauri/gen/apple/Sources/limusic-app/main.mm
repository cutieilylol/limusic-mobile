#include "bindings/bindings.h"

// LimusicBridge.swift: connects the Rust bridge before anything can send on it.
extern "C" void limusic_native_start();

int main(int argc, char * argv[]) {
	limusic_native_start();
	ffi::start_app();
	return 0;
}

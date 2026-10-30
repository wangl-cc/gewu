.DEFAULT_GOAL := app

CONFIG ?= release
BUILD_NUMBER ?= 1
CODEC ?= none

DERIVED_SOURCES = build/xcode/Build/Intermediates.noindex/Gewu.build/$(XCODE_CONFIG)/Gewu.build/DerivedSources
GEWU_BENCH_FILE ?= target/bench/sample.parquet

ifeq ($(CONFIG),debug)
XCODE_CONFIG = Debug
else ifeq ($(CONFIG),release)
XCODE_CONFIG = Release
else ifeq ($(CONFIG),release-lto)
XCODE_CONFIG = Release-LTO
else
$(error CONFIG must be debug, release or release-lto)
endif

APP = build/$(XCODE_CONFIG)/Gewu.app
XCODEBUILD = xcodebuild -quiet -project Gewu.xcodeproj -scheme Gewu \
	-configuration $(XCODE_CONFIG) -destination 'generic/platform=macOS' \
	-derivedDataPath "$(CURDIR)/build/xcode" SYMROOT="$(CURDIR)/build" CURRENT_PROJECT_VERSION=$(BUILD_NUMBER)

.PHONY: project app run dist test test-swift fmt fmt-rust fmt-swift fmt-check fmt-check-rust lint lint-rust lint-swift deny check-features bench-sample bench bench-mem help

project:
	xcodegen generate --use-cache

app: project
	$(XCODEBUILD) build

run:
	open $(APP)

dist: app
	lipo $(APP)/Contents/MacOS/Gewu -verify_arch $(shell uname -m)
	plutil -lint $(APP)/Contents/Info.plist
	codesign --verify --deep --strict $(APP)
	ditto -c -k --sequesterRsrc --keepParent $(APP) build/Gewu-macos.zip

test:
	cargo test --locked --workspace

$(GEWU_BENCH_FILE): tools/data-gen/src/main.rs tools/data-gen/Cargo.toml Cargo.lock
	mkdir -p "$(@D)"
	cargo run --locked --release -p gewu-data-gen -- "$@" 2000000

bench-sample: $(GEWU_BENCH_FILE)

bench: bench-sample
	GEWU_BENCH_FILE="$(abspath $(GEWU_BENCH_FILE))" cargo bench --locked -p gewu-core --bench engine

bench-mem: bench-sample
	cargo build --locked --release -p gewu-cli
	scripts/bench-mem.sh "target/release/gewu" "$(abspath $(GEWU_BENCH_FILE))"

test-swift: app
	xcrun --sdk macosx swiftc -parse-as-library \
		-Xcc -fmodule-map-file="$(DERIVED_SOURCES)/gewuFFI.modulemap" \
		"$(DERIVED_SOURCES)/gewu.swift" app/Sources/Gewu/Table/RowCache.swift \
		app/Sources/Gewu/Table/CopyModel.swift app/Tests/TableExecutionTests.swift \
		"build/$(XCODE_CONFIG)/libgewu.a" -liconv -o build/table-execution-tests
	build/table-execution-tests
	xcrun --sdk macosx swiftc -parse-as-library \
		-Xcc -fmodule-map-file="$(DERIVED_SOURCES)/gewuFFI.modulemap" \
		"$(DERIVED_SOURCES)/gewu.swift" \
		$$(rg --files app/Sources/Gewu -g '*.swift' -g '!GewuApp.swift' | sort) \
		app/Tests/WorkbookLifecycleTests.swift app/Tests/ChartPresentationTests.swift app/Tests/FileOpeningTests.swift \
		"build/$(XCODE_CONFIG)/libgewu.a" -liconv -o build/workbook-lifecycle-tests
	build/workbook-lifecycle-tests

fmt: fmt-rust fmt-swift

fmt-rust:
	cargo +nightly fmt --all

fmt-swift:
	xcrun swift-format format --configuration .swift-format --in-place --recursive app/Sources/Gewu

fmt-check: fmt-check-rust lint-swift

fmt-check-rust:
	cargo +nightly fmt --all --check

lint: lint-rust lint-swift

lint-rust:
	cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

lint-swift:
	xcrun swift-format lint --configuration .swift-format --strict --recursive app/Sources/Gewu

deny:
	cargo deny --locked check

check-features:
	@case "$(CODEC)" in none|parquet|csv|xlsx) ;; *) echo 'CODEC must be none, parquet, csv or xlsx' >&2; exit 2 ;; esac
	cargo test --locked -p gewu-core --no-default-features $(if $(filter-out none,$(CODEC)),--features $(CODEC))
	cargo clippy --locked -p gewu-core --all-targets --no-default-features $(if $(filter-out none,$(CODEC)),--features $(CODEC)) -- -D warnings

help:
	@echo 'make app [CONFIG=debug|release|release-lto] [BUILD_NUMBER=1]'
	@echo 'make project   Generate Gewu.xcodeproj for Xcode'
	@echo 'make run       Open the existing Gewu.app without building'
	@echo 'make app run   Build and then open Gewu.app (without -j)'
	@echo 'make dist      Build, verify and archive build/Gewu-macos.zip'
	@echo 'make bench-sample | bench | bench-mem [GEWU_BENCH_FILE=path]'
	@echo 'make fmt       Format Rust and Swift sources'
	@echo 'make fmt-check Check Rust formatting and Swift formatting/style'
	@echo 'make lint      Run Rust Clippy and Swift formatting/style checks'
	@echo 'make fmt-rust | fmt-swift | fmt-check-rust | lint-rust | lint-swift'
	@echo 'make test-swift Run headless table, worksheet and chart regressions against the built app core'
	@echo 'make test | deny | check-features CODEC=none|parquet|csv|xlsx'

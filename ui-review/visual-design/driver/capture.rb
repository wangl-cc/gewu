#!/usr/bin/env ruby
# Build an isolated capture entry point against this checkout and its Cargo.lock.
require "digest"
require "fileutils"
require "json"
require "open3"
require "optparse"
require "pathname"
require "tmpdir"

require_relative "scenes"

driver = Pathname.new(__dir__)
repo = driver.join("../../..").realpath
output = driver.parent
sample = nil
modes = %w[standard high-contrast]
groups = []
OptionParser.new do |opts|
  opts.banner = "Usage: ruby capture.rb [options]"
  opts.on("--sample FILE", "Use an existing gewu-data-gen 2M-row Parquet") { |p| sample = File.expand_path(p) }
  opts.on("--output DIR", "Set the screenshot destination") { |p| output = Pathname.new(p).expand_path }
  opts.on("--mode MODE", %w[standard high-contrast], "Capture one mode") { |m| modes = [m] }
  opts.on("--group GROUP", Scene::GROUPS, "Repeat to select scene groups") { |g| groups << g }
end.parse!
groups = Scene::GROUPS if groups.empty?
abort "This capture host needs macOS and Metal." unless RUBY_PLATFORM.include?("darwin")

Dir.chdir(repo)
# Cargo locates both the pinned GPUI source and the checkout's configured target directory.
triple = `rustc -vV`[/^host: (.+)$/, 1] or abort "could not find the Rust host target"
metadata, status = Open3.capture2("cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", triple)
abort "cargo metadata failed" unless status.success?
cargo = JSON.parse(metadata)
target = Pathname.new(cargo.fetch("target_directory"))
work = target.join("visual-design-capture")
FileUtils.mkdir_p(work)
engine = cargo.fetch("packages").find { |p| p["name"] == "gpui-fast" }
patch = driver.join("headless-controls.patch")
engine_src = Pathname.new(engine.fetch("manifest_path")).dirname
engine_copy = work.join("gpui-fast-#{engine.fetch("version")}-#{Digest::SHA256.file(patch).hexdigest[0, 12]}")
unless engine_copy.directory?
  Dir.mktmpdir("engine-", work.to_s) do |tmp|
    copy = File.join(tmp, "source")
    FileUtils.cp_r(engine_src.to_s, copy)
    abort "GPUI headless adapter patch failed" unless system("patch", "-d", copy, "-p1", "-i", patch.to_s)
    File.rename(copy, engine_copy)
  end
end

# Preserve the workspace dependency graph. Only the capture binary and the two
# test-platform observations differ from the real application build.
shell = repo.join("gpui/src/shell.rs")
src = work.join("gpui/src")
FileUtils.mkdir_p(src)
%w[crates tools gpui/macos].each do |link|
  dest = work.join(link)
  File.symlink(repo.join(link), dest) unless dest.symlink?
end
File.write(work.join("Cargo.toml"), repo.join("Cargo.toml").read + "\n[patch.crates-io]\ngpui-fast = { path = #{engine_copy.to_s.to_json} }\n")
FileUtils.cp(repo.join("Cargo.lock"), work.join("Cargo.lock"))
manifest = repo.join("gpui/Cargo.toml").read
manifest = manifest.sub('name = "gewu-gpui"', 'name = "gewu-design-capture"')
manifest = manifest.sub('[dependencies]', <<~TOML.chomp)
  [dependencies]
  accesskit_consumer = "0.38"
  serde_json = "1"
  image = { version = "0.25", default-features = false, features = ["png"] }
TOML
manifest = manifest.sub('default = ["gpui-fast"]', 'default = ["gpui-fast", "gpui-kit/test-support"]')
File.write(work.join("gpui/Cargo.toml"), manifest)
repo.join("gpui/src").children.each do |entry|
  next if %w[main.rs lib.rs shell.rs].include?(entry.basename.to_s)
  link = src.join(entry.basename)
  File.symlink(entry, link) unless link.symlink?
end
File.write(src.join("main.rs"), repo.join("gpui/src/lib.rs").read + "\nfn main() { shell::review::run(); }\n")
File.write(src.join("shell.rs"), shell.read + "\n#[path = \"review.rs\"] pub(crate) mod review;\n")
FileUtils.cp(driver.join("host.rs"), src.join("review.rs"))
binary = target.join("debug/gewu-design-capture")
abort "could not build screenshot host" unless system("cargo","build","--offline","--manifest-path", work.join("Cargo.toml").to_s,"--target-dir",target.to_s,"-p","gewu-design-capture")
if groups.include?("large") && sample.nil?
  sample = work.join("sample.parquet").to_s
  unless File.file?(sample)
    abort "could not generate large fixture" unless system("cargo", "run", "--locked", "-p", "gewu-data-gen", "--", sample, "2000000")
  end
end
harbor = driver.join("fixtures/Harbor observations.csv")
corrupt = work.join("Damaged observations.parquet")
File.write(corrupt, "Incomplete example data")
modes.each do |mode|
  Scene.run_groups(groups, binary.to_s, harbor.to_s, corrupt.to_s, sample, output.to_s, mode)
end

# frozen_string_literal: true

desc 'Check the formatting, run the tests and clippy'
task :cargo do
  sh 'cargo fmt --check'
  sh 'cargo test --quiet'
  sh 'cargo clippy --quiet --all-targets -- -D warnings'
end

task default: :cargo

LINUX_DIST = {
  'x86_64-unknown-linux-musl' => 'charta-linux-x86_64',
  'aarch64-unknown-linux-musl' => 'charta-linux-arm64',
}.freeze
MACOS_TARGETS = %w[x86_64-apple-darwin aarch64-apple-darwin].freeze
LINUX = RUBY_PLATFORM.include?('linux')
MACOS = RUBY_PLATFORM.include?('darwin')

def release_build(targets, command = 'build')
  sh 'rustup', 'target', 'add', *targets
  targets.each { |target| sh 'cargo', command, '--release', '--target', target }
  mkdir_p 'dist'
end

def available?(*command)
  system(*command, out: File::NULL, err: File::NULL)
end

def release_binary(target)
  "target/#{target}/release/charta"
end

namespace :dist do
  desc 'Build static Linux binaries into dist/ (on Linux only, with zig and cargo-zigbuild)'
  task :linux do
    abort 'dist:linux needs Linux, build the Linux binaries there' unless LINUX
    unless available?('zig', 'version') && available?('cargo', 'zigbuild', '--help')
      abort 'dist:linux needs zig and cargo-zigbuild: mise use -g zig cargo:cargo-zigbuild'
    end
    # cargo-zigbuild uses zig to link against musl for both architectures
    release_build(LINUX_DIST.keys, 'zigbuild')
    LINUX_DIST.each { |target, asset| cp release_binary(target), "dist/#{asset}" }
  end

  desc 'Build a universal macOS binary into dist/ (on macOS only)'
  task :macos do
    abort 'dist:macos needs macOS for its SDK and lipo' unless MACOS
    release_build(MACOS_TARGETS)
    sh 'lipo', '-create', '-output', 'dist/charta-macos', *MACOS_TARGETS.map { |target| release_binary(target) }
  end
end

desc "Build this platform's release binaries into dist/: Linux on Linux, macOS on a Mac"
task dist: MACOS ? 'dist:macos' : 'dist:linux' do
  # Each platform rebuilds only its own binaries, the others may be from an older version
  stale = Dir['dist/*'].select { |asset| File.mtime(asset) < File.mtime('Cargo.toml') }
  warn "Older than Cargo.toml, rebuild them on their platform: #{stale.join(', ')}" unless stale.empty?
end

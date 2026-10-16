#!/usr/bin/env bash
# Prints the folder holding libclang (which bindgen needs for the FFmpeg
# bindings) on Linux and macOS. .cargo/config.toml defaults LIBCLANG_PATH to
# the Windows LLVM folder, so other systems must set it:
#
#   export LIBCLANG_PATH="$(bash scripts/libclang-path.sh)"
set -eu

has_libclang() {
  local file
  for file in "$1"/libclang.so* "$1"/libclang-*.so* "$1"/libclang.dylib; do
    [ -e "$file" ] && return 0
  done
  return 1
}

candidates=()
if command -v llvm-config >/dev/null 2>&1; then
  candidates+=("$(llvm-config --libdir)")
fi
case "$(uname -s)" in
  Darwin)
    developer="$(xcode-select -p 2>/dev/null || true)"
    candidates+=(
      "$developer/Toolchains/XcodeDefault.xctoolchain/usr/lib"
      "$developer/usr/lib"
      /Library/Developer/CommandLineTools/usr/lib
    )
    if command -v brew >/dev/null 2>&1 && brew --prefix llvm >/dev/null 2>&1; then
      candidates+=("$(brew --prefix llvm)/lib")
    fi
    ;;
  *)
    # Distribution packages (newest LLVM first), then the usual library folders.
    for dir in $(ls -d /usr/lib/llvm-*/lib 2>/dev/null | sort -V -r); do
      candidates+=("$dir")
    done
    candidates+=(/usr/lib64 /usr/lib/x86_64-linux-gnu /usr/lib/aarch64-linux-gnu /usr/lib)
    ;;
esac

for dir in "${candidates[@]}"; do
  if [ -n "$dir" ] && has_libclang "$dir"; then
    echo "$dir"
    exit 0
  fi
done
echo "libclang not found: install clang (Linux: libclang-dev; macOS: Xcode command line tools)" >&2
exit 1

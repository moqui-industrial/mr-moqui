# Source this before building for wasm32-unknown-emscripten (plans/archive/Wasm.md).
#   . scripts/wasm-env.sh && cargo check -p moqui-core --no-default-features --target wasm32-unknown-emscripten
# EMSDK defaults to ~/emsdk. LIBCLANG_PATH defaults to the newest /usr/lib/llvm-*/lib.
: "${EMSDK:=$HOME/emsdk}"
. "$EMSDK/emsdk_env.sh" >/dev/null 2>&1 || { echo "emsdk not found at $EMSDK" >&2; return 1; }
if [ -z "${LIBCLANG_PATH:-}" ]; then
    LIBCLANG_PATH=$(ls -d /usr/lib/llvm-*/lib 2>/dev/null | sort -V | tail -1)
    export LIBCLANG_PATH
fi
# Builtin headers (stdbool.h, stddef.h) from the clang emcc ships, not the host clang.
_clang_inc=$(ls -d "$EMSDK"/upstream/lib/clang/*/include 2>/dev/null | sort -V | tail -1)
_sysroot="$EMSDK/upstream/emscripten/cache/sysroot"
# rquickjs-sys bindgen: emscripten headers, and default visibility so wasm clang emits the functions.
export BINDGEN_EXTRA_CLANG_ARGS_wasm32_unknown_emscripten="--sysroot=$_sysroot -isystem $_sysroot/include -isystem $_clang_inc -fvisibility=default"
unset _clang_inc _sysroot

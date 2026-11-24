# ClusterFuzzLite build image for the cargo-fuzz targets in fuzz/ (.github/workflows/fuzz.yml).
# The OSS-Fuzz Rust builder, pinned by digest: a nightly toolchain and cargo-fuzz.
FROM gcr.io/oss-fuzz-base/base-builder-rust:v1@sha256:d345218878c78feed410f0ad57268617a368912b224f844bbdb71c54c0b9955a
COPY . $SRC/graphviz-pure
WORKDIR $SRC/graphviz-pure
COPY .clusterfuzzlite/build.sh $SRC/

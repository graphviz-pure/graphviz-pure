# The graphviz-pure container image: a release's static musl binary and its licence files,
# nothing else. scripts/release-image.sh builds it from the release archives; it cannot be built
# from a checkout. The binary needs no files at runtime (the fonts for raster output are compiled in)
# and never opens a network connection, so the image has no base, shell, libc or CA bundle.
FROM scratch
ARG TARGETARCH
ARG VERSION
ARG REVISION
ARG CREATED
LABEL org.opencontainers.image.title="graphviz-pure" \
      org.opencontainers.image.description="A pure-Rust port of Graphviz with the same output on every platform: a drop-in dot command line" \
      org.opencontainers.image.source="https://github.com/graphviz-pure/graphviz-pure" \
      org.opencontainers.image.url="https://graphviz-pure.github.io/" \
      org.opencontainers.image.licenses="EPL-2.0" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.created="${CREATED}"
# LICENSE, NOTICE, THIRD-PARTY-LICENSES.md and LICENSES/, from the release archive.
COPY doc/ /usr/share/doc/graphviz-pure/
COPY ${TARGETARCH}/ /usr/local/bin/
# Mount the files to render here: docker run -v "$PWD:/data" ... -o out.svg in.dot
WORKDIR /data
# nobody: the binary needs no privileges. Pass --user "$(id -u):$(id -g)" to write into a mount.
USER 65534:65534
ENTRYPOINT ["/usr/local/bin/graphviz-pure"]

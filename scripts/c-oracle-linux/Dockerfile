# C Graphviz 16.1.0 on Linux, built like the macOS reference (Homebrew's build, used through
# a plugin directory with only the core, dot and neato plugins so C measures text with its
# built-in font tables; scripts/gen-golden.sh): clang, LLVM's libc++ for the C++ parts, and
# GTS, which neato and sfdp use for triangulation (overlap removal, multispline routing).
# Used to measure how far C on Linux is from C on macOS, and how far the port's portable build
# is from C on Linux (docs/porting/libm.md). On x86-64, with the port's math and Apple's libc
# (run.sh's apple-libc run), it reproduces the macOS references on all but 7 of 2595 inputs.
#
#   docker build -t gv-c-linux -f scripts/c-oracle-linux/Dockerfile <dir holding graphviz-16.1.0.tar.xz>
#   scripts/c-oracle-linux/run.sh
FROM rust:1-bookworm@sha256:5c1a1732ac3f77635b24d397ceb67b4d9ea61de348f68c0aa54a79fd01e65c69
RUN apt-get update && apt-get install -y --no-install-recommends \
      clang libc++-dev libc++abi-dev libgts-dev bison flex pkg-config libexpat1-dev zlib1g-dev \
      libltdl-dev xz-utils \
    && rm -rf /var/lib/apt/lists/*
COPY graphviz-16.1.0.tar.xz /src/
# APPLE_LONG_DOUBLE=1 makes neato's stress majorization accumulate in double, as it does on
# Apple arm64, where long double is double (on Linux aarch64 it is a 128-bit quad).
ARG APPLE_LONG_DOUBLE=0
# On x86-64, -mfma: clang contracts a*b+c into a fused multiply-add (-ffp-contract=on, its
# default) only for a target with an FMA instruction, which arm64 has and baseline x86-64 lacks.
# Without it C on x86-64 fuses nothing and differs from the macOS reference on about 500 inputs.
RUN cd /src && tar xf graphviz-16.1.0.tar.xz && cd graphviz-16.1.0 \
    && if [ "$APPLE_LONG_DOUBLE" = 1 ]; then \
         sed -i 's/^#define DegType long double$/#define DegType double/' lib/neatogen/stress.c \
         && grep -q '^#define DegType double$' lib/neatogen/stress.c; fi \
    && FLAGS=-O2 && if [ "$(uname -m)" = x86_64 ]; then FLAGS="-O2 -mfma"; fi \
    && CC=clang CXX="clang++ -stdlib=libc++" CFLAGS="$FLAGS" CXXFLAGS="$FLAGS" ./configure --prefix=/opt/gv \
         --with-gts --without-pangocairo --without-gdk --without-gdk-pixbuf --without-gtk \
         --without-x --without-gd --without-rsvg --without-ghostscript --without-poppler \
         --without-webp --without-lasi --without-qt --disable-swig --disable-tcl \
         --disable-perl --disable-python --disable-man-pdfs \
    && grep -q '^#define HAVE_GTS 1' config.h \
    && make -j"$(nproc)" && make install
RUN mkdir /opt/gv/lutplugins \
    && cp -P /opt/gv/lib/graphviz/libgvplugin_core.so* /opt/gv/lib/graphviz/libgvplugin_dot_layout.so* \
         /opt/gv/lib/graphviz/libgvplugin_neato_layout.so* /opt/gv/lutplugins/ \
    && GVBINDIR=/opt/gv/lutplugins /opt/gv/bin/dot -c
ENV PATH=/opt/gv/bin:$PATH GVBINDIR=/opt/gv/lutplugins

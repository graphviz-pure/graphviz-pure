# The image of graphviz-pure's self-hosted Linux runners (docs/ci/self-hosted-runners.md).
#
# It stands in for GitHub's ubuntu-24.04 image for the jobs of ci.yml and cross-platform.yml:
# the same Ubuntu release and the tools those jobs call without installing them. It is not a
# copy of GitHub's image. A job that needs a tool missing here fails, and the tool is added here.
# Nothing that changes a job's output is added unless the hosted image has it too (no wasm-opt,
# for example: scripts/site-wasm.sh and web/ skip it when absent, as on the hosted runner).
#
# Build: scripts/ci/runner/build.sh (tags gvp-ci-runner:<runner version>).

FROM ubuntu:24.04@sha256:fe81560990fae86dc4743202b2cb4d3df98d0bcf949173cecef15ce1127ef852

ARG RUNNER_VERSION=2.338.2
ARG RUNNER_SHA256=baebf204307bac6193480852732f0aba2ae97b6c5597c244f4d5558390147c40
ARG GH_VERSION=2.103.3
ARG GH_SHA256=4c806978ffce1cec70853302a56d70ac6da4f19bae7adca4e1a8ab00633dd91d
# The hosted image's default Node major is on PATH; jobs that pin one use actions/setup-node.
ARG NODE_VERSION=20.20.2
ARG NODE_SHA256=df770b2a6f130ed8627c9782c988fda9669fa23898329a61a871e32f965e007d
# Google's Linux package signing key (the hosted image installs Chrome from the same repository).
ARG CHROME_KEY_FINGERPRINT=EB4C1BFD4F042F6DDDCCEC917721F63BD38B4796

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    LC_ALL=C.UTF-8

# The tools the Linux jobs call without installing: a C toolchain for rustc's linker and build
# scripts, python3, jq, git, curl, unzip, zstd (actions/cache), column and bc, plus the packages
# that jobs install with `sudo apt-get install` (musl-tools, qemu-user, valgrind), preinstalled
# because the job user has no root; see the sudo shim below.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential pkg-config ca-certificates curl wget git git-lfs gnupg jq \
      python3 python3-venv python3-pip unzip zip xz-utils zstd bzip2 file \
      bsdextrautils bc less lsb-release locales openssh-client patch rsync tzdata time \
      musl-tools qemu-user valgrind \
      libnss3 libatk-bridge2.0-0t64 libgtk-3-0t64 libgbm1 libasound2t64 libxss1 fonts-liberation xdg-utils \
 && rm -rf /var/lib/apt/lists/*

# Google Chrome, which scripts/wasm-runtimes/run.mjs launches through Playwright's `chrome`
# channel, as on the hosted image. The key is checked against its published fingerprint.
RUN curl -fsSL https://dl.google.com/linux/linux_signing_key.pub -o /tmp/google.pub \
 && fpr=$(gpg --show-keys --with-colons /tmp/google.pub | awk -F: '/^fpr/ { print $10; exit }') \
 && test "$fpr" = "$CHROME_KEY_FINGERPRINT" \
 && gpg --dearmor -o /usr/share/keyrings/google-chrome.gpg /tmp/google.pub \
 && rm /tmp/google.pub \
 && echo "deb [arch=amd64 signed-by=/usr/share/keyrings/google-chrome.gpg] https://dl.google.com/linux/chrome/deb/ stable main" \
      > /etc/apt/sources.list.d/google-chrome.list \
 && apt-get update \
 && apt-get install -y --no-install-recommends google-chrome-stable \
 && rm -rf /var/lib/apt/lists/*

# The GitHub CLI (scripts/ci-result-cache.sh and scripts/ci-report-failure.sh call gh).
RUN curl -fsSL -o /tmp/gh.tgz "https://github.com/cli/cli/releases/download/v${GH_VERSION}/gh_${GH_VERSION}_linux_amd64.tar.gz" \
 && echo "${GH_SHA256}  /tmp/gh.tgz" | sha256sum --check --strict \
 && tar -xzf /tmp/gh.tgz -C /tmp \
 && install -m 0755 "/tmp/gh_${GH_VERSION}_linux_amd64/bin/gh" /usr/local/bin/gh \
 && rm -rf /tmp/gh.tgz "/tmp/gh_${GH_VERSION}_linux_amd64"

# Node.js on PATH, as on the hosted image (scripts/tests run node without setup-node).
RUN curl -fsSL -o /tmp/node.tar.xz "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" \
 && echo "${NODE_SHA256}  /tmp/node.tar.xz" | sha256sum --check --strict \
 && tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1 --exclude='*.md' --exclude=LICENSE \
 && rm /tmp/node.tar.xz \
 && node --version

# The job user, named and placed like the hosted runner's (/home/runner/work/<repo>/<repo>), so
# paths in caches and outputs match. It has no root and no sudo.
RUN userdel -r ubuntu 2>/dev/null || true \
 && useradd --create-home --uid 1001 --shell /bin/bash runner

# The Actions runner, pinned and checked. Its own dependencies come from its installer script.
RUN mkdir -p /opt/actions-runner \
 && curl -fsSL -o /tmp/runner.tgz "https://github.com/actions/runner/releases/download/v${RUNNER_VERSION}/actions-runner-linux-x64-${RUNNER_VERSION}.tar.gz" \
 && echo "${RUNNER_SHA256}  /tmp/runner.tgz" | sha256sum --check --strict \
 && tar -xzf /tmp/runner.tgz -C /opt/actions-runner \
 && rm /tmp/runner.tgz \
 && apt-get update \
 && /opt/actions-runner/bin/installdependencies.sh \
 && rm -rf /var/lib/apt/lists/* \
 && chown -R runner:runner /opt/actions-runner

# `sudo apt-get update` and `sudo apt-get install <preinstalled packages>` succeed without doing
# anything; every other use of sudo fails. The jobs' apt steps stay as they are for the hosted
# runners.
COPY sudo-shim.sh /usr/local/bin/sudo
COPY job-started.sh /opt/gvp-ci/job-started.sh
COPY entrypoint.sh /opt/gvp-ci/entrypoint.sh
RUN chmod 0755 /usr/local/bin/sudo /opt/gvp-ci/job-started.sh /opt/gvp-ci/entrypoint.sh

USER runner
WORKDIR /home/runner

# rustup, as on the hosted image (~/.cargo/bin on PATH). Each job installs the toolchain it
# names; stable is preinstalled so that install is usually a check. rustup-init is checked
# against the sha256 published beside it.
# CARGO_HOME and RUSTUP_HOME stay unset, as on the hosted image: Swatinem/rust-cache hashes them
# into its cache key, and setting them would keep these runners from sharing the hosted caches.
ENV PATH=/home/runner/.cargo/bin:$PATH
RUN curl -fsSL -o /tmp/rustup-init https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init \
 && curl -fsSL https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init.sha256 \
      | awk '{ print $1 "  /tmp/rustup-init" }' | sha256sum --check --strict \
 && chmod +x /tmp/rustup-init \
 && /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain stable \
 && rm /tmp/rustup-init

# The hosted image's identifiers, which some actions read.
ENV ImageOS=ubuntu24 \
    RUNNER_TOOL_CACHE=/home/runner/work/_tool \
    AGENT_TOOLSDIRECTORY=/home/runner/work/_tool \
    ACTIONS_RUNNER_HOOK_JOB_STARTED=/opt/gvp-ci/job-started.sh

# The entrypoint starts as root only to write /etc/resolv.conf, then becomes `runner`.
USER root
WORKDIR /home/runner
ENTRYPOINT ["/opt/gvp-ci/entrypoint.sh"]

FROM ubuntu:24.04

# Avoid prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive

# Install core development tools, curl, git, build-essential
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    curl \
    git \
    gnupg \
    jq \
    pkg-config \
    libssl-dev \
    python3 \
    python3-pip \
    python3-venv \
    sudo \
    tzdata \
    unzip \
    wget \
    gh \
    && rm -rf /var/lib/apt/lists/*

# Ubuntu 24.04 includes the non-root 'ubuntu' user (UID 1000, GID 1000)
ARG USERNAME=ubuntu

RUN echo "$USERNAME ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/$USERNAME \
    && chmod 0440 /etc/sudoers.d/$USERNAME

# Install Node.js 22 LTS
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y nodejs \
    && corepack enable \
    && rm -rf /var/lib/apt/lists/*

# Switch to developer user to install user-space toolchains (Rust, Python venv)
USER $USERNAME
ENV HOME=/home/$USERNAME
WORKDIR $HOME

# Install Rust toolchain (stable) via rustup
ENV RUSTUP_HOME=$HOME/.rustup \
    CARGO_HOME=$HOME/.cargo \
    PATH=$HOME/.cargo/bin:$PATH

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable \
    && rustup component add rustfmt clippy

# Create isolated Python virtual environment for governance tools
RUN python3 -m venv $HOME/.venv \
    && $HOME/.venv/bin/pip install --upgrade pip \
    && $HOME/.venv/bin/pip install jsonschema

ENV PATH=$HOME/.venv/bin:$PATH

# Mark workspace as safe git directory
RUN git config --global --add safe.directory /workspace

# Default workspace directory inside container
WORKDIR /workspace
CMD ["/bin/bash"]

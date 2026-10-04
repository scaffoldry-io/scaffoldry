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
    && rm -rf /var/lib/apt/lists/*

# Create a non-root developer user: developer (uid 1000)
ARG USERNAME=developer
ARG USER_UID=1000
ARG USER_GID=1000

RUN groupadd --gid $USER_GID $USERNAME \
    && useradd --uid $USER_UID --gid $USER_GID -m -s /bin/bash $USERNAME \
    && echo "$USERNAME ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/$USERNAME \
    && chmod 0440 /etc/sudoers.d/$USERNAME

# Install GitHub CLI (gh)
RUN curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg | dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg \
    && chmod go+r /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" | tee /etc/apt/sources.list.d/github-cli.list > /dev/null \
    && apt-get update && apt-get install -y github-cli \
    && rm -rf /var/lib/apt/lists/*

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

# Default workspace directory inside container
WORKDIR /workspace
CMD ["/bin/bash"]

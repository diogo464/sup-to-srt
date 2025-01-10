FROM docker.io/rust:1.84-slim AS builder
WORKDIR /usr/src/
COPY . .
RUN apt update && apt install -y pkg-config libclang-dev libleptonica-dev libtesseract-dev
RUN cargo install --no-default-features --path .

FROM docker.io/debian:stable-slim
COPY --from=builder /usr/local/cargo/bin/sup-to-srt /usr/bin/sup-to-srt
RUN apt update && apt install -y tesseract-ocr && apt-get clean && rm -rf /var/lib/apt/lists/*
ENTRYPOINT ["/usr/bin/sup-to-srt"]


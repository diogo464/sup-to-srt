FROM docker.io/rust:1.94.1-slim-trixie AS builder
WORKDIR /usr/src/
COPY . .
RUN apt update && apt install -y pkg-config libclang-dev libleptonica-dev libtesseract-dev
RUN cargo install --no-default-features --path .

FROM docker.io/debian:stable-slim
COPY --from=builder /usr/local/cargo/bin/sup-to-srt /usr/bin/sup-to-srt
RUN apt update && apt install -y \
    tesseract-ocr \
    tesseract-ocr-ara \
    tesseract-ocr-chi-sim \
    tesseract-ocr-chi-tra \
    tesseract-ocr-deu \
    tesseract-ocr-fra \
    tesseract-ocr-ita \
    tesseract-ocr-jpn \
    tesseract-ocr-kor \
    tesseract-ocr-nld \
    tesseract-ocr-por \
    tesseract-ocr-rus \
    tesseract-ocr-spa \
    && apt-get clean && rm -rf /var/lib/apt/lists/*
ENTRYPOINT ["/usr/bin/sup-to-srt"]


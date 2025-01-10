# sup-to-srt

`sup-to-srt` is a tool that converts PGS subtitles (`.sup` files) to SRT subtitles using Tesseract OCR. 

## Quick Start with Docker
To run the program using Docker, simply pipe your `.sup` file to the container:

```bash
cat subtitles.sup | docker run --rm -i ghcr.io/diogo464/sup-to-srt:latest > subtitles.srt
```

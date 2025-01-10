#!/usr/bin/env bash

# Default values
DEFAULT_TAG="git.d464.sh/diogo464/sup-to-srt:latest"
TAG="${TAG:-$DEFAULT_TAG}"

# Build the Docker image
echo "Building Docker image with tag: $TAG"
docker build -f Containerfile -t "$TAG" .

# Check if the image should be pushed
if [ "$PUSH" == "1" ]; then
  echo "Pushing Docker image: $TAG"
  docker push "$TAG"
else
  echo "PUSH is not set to 1, skipping image push."
fi


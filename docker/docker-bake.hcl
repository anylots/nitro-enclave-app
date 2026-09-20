# Run from the repository root.
target "enclave" {
  context = "."
  dockerfile = "docker/Dockerfile.enclave"
  tags = ["nitro-enclave-app:local"]
  platforms = ["linux/amd64"]
}

target "nitro-kernel" {
  context = "https://github.com/aws/aws-nitro-enclaves-sdk-bootstrap.git#f718dea60a9d9bb8b8682fd852ad793912f3c5db"
  target = "artifacts"
  args = {
    TARGET = "kernel"
  }
  platforms = ["linux/amd64"]
}

target "eif-builder" {
  context = "."
  dockerfile = "docker/Dockerfile.eif-builder"
  contexts = {
    nitro-kernel = "target:nitro-kernel"
  }
  tags = ["nitro-enclave-eif-builder:local"]
  platforms = ["linux/amd64"]
}


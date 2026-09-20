# Nitro Enclave EIF build

This pipeline packages `nitro-enclave-app` as an Enclave Image File (EIF):

```text
Rust release binary -> application Docker image -> Nitro CLI -> EIF + PCR measurements
```

## Prerequisites

- Linux with Docker and Docker Buildx/Bake
- Access to `/var/run/docker.sock`
- Network access for container images, Rust crates, and the pinned AWS bootstrap source

All images target `linux/amd64`. A non-amd64 build host therefore needs amd64
emulation. An enclave-enabled EC2 instance is only required to run the EIF, not
to build it.

## Build

Run from any directory:

```sh
bash /path/to/nitro-enclave-app/docker/build-eif.sh
```

On success, both outputs are published atomically under:

```text
target/enclave-eif/enclave.eif
target/enclave-eif/measurements.json
```

`enclave.eif` is the launchable image. `measurements.json` contains Nitro
CLI's PCR0/PCR1/PCR2 measurements; PCR0 identifies this exact image during
attestation verification.

To build and load only the application and packaging images:

```sh
docker buildx bake -f docker/docker-bake.hcl --load enclave eif-builder
```

The builder uses the Linux 6.6.79 kernel and matching NSM module from a pinned
revision of AWS's Nitro Enclaves SDK bootstrap repository.

## Run on AWS

Copy `target/enclave-eif/enclave.eif` to an enclave-enabled x86_64 EC2 parent
instance with Nitro CLI installed. After allocating capacity in
`/etc/nitro_enclaves/allocator.yaml`, start it with:

```sh
sudo systemctl restart nitro-enclaves-allocator.service
nitro-cli run-enclave \
  --enclave-name nitro-enclave-app \
  --cpu-count 2 \
  --memory 2048 \
  --enclave-cid 16 \
  --eif-path target/enclave-eif/enclave.eif
nitro-cli describe-enclaves
```

The application starts automatically and listens on VSOCK port `8000`.
Increase enclave memory for large requests; the application permits frames up
to 512 MiB and recommends budgeting roughly 2.5 times the largest encoded
request plus runtime overhead.


FROM ubuntu:24.04 AS release
ARG YASHY_VERSION=1.0.0
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*
# install.sh verifies the release checksum before extracting the archive.
RUN curl -fsSL https://github.com/Yash-K-Jagani/Yashy/releases/download/v${YASHY_VERSION}/install.sh \
      -o /tmp/install.sh \
 && sh /tmp/install.sh --prefix /usr/local --no-path --no-doctor \
 && rm /tmp/install.sh

FROM scratch
COPY --from=release /usr/local/bin/yashyc /usr/local/bin/yashyc
COPY --from=release /usr/local/share/yashy/stdlib /usr/local/share/yashy/stdlib
ENV PATH=/usr/local/bin:$PATH
ENTRYPOINT ["/usr/local/bin/yashyc"]

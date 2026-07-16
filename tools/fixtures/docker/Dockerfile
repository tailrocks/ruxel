FROM debian:12-slim

RUN apt-get update -qq \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq openssh-server python3 \
 && rm -rf /var/lib/apt/lists/* \
 && mkdir -p /run/sshd /root/.ssh \
 && chmod 0700 /root/.ssh \
 && ssh-keygen -A \
 && printf 'PermitRootLogin prohibit-password\nPasswordAuthentication no\n' \
      >/etc/ssh/sshd_config.d/ruxel-fixture.conf

EXPOSE 22
CMD ["/usr/sbin/sshd", "-D", "-e"]

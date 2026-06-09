from __future__ import annotations as _annotations

import ipaddress
import os
import socket
from urllib.parse import urlparse

__all__ = (
    "SsrfBlockedError",
    "resolve_allowed_host",
    "validate_outbound_url",
)


class SsrfBlockedError(ValueError):
    """Raised when an outbound HTTP URL targets a blocked network."""


def _blocked_networks() -> tuple[ipaddress._BaseNetwork, ...]:  # noqa: SLF001
    return (
        ipaddress.ip_network("0.0.0.0/8"),
        ipaddress.ip_network("10.0.0.0/8"),
        ipaddress.ip_network("127.0.0.0/8"),
        ipaddress.ip_network("169.254.0.0/16"),
        ipaddress.ip_network("172.16.0.0/12"),
        ipaddress.ip_network("192.0.0.0/24"),
        ipaddress.ip_network("192.0.2.0/24"),
        ipaddress.ip_network("192.168.0.0/16"),
        ipaddress.ip_network("198.18.0.0/15"),
        ipaddress.ip_network("198.51.100.0/24"),
        ipaddress.ip_network("203.0.113.0/24"),
        ipaddress.ip_network("224.0.0.0/4"),
        ipaddress.ip_network("240.0.0.0/4"),
        ipaddress.ip_network("::1/128"),
        ipaddress.ip_network("fc00::/7"),
        ipaddress.ip_network("fe80::/10"),
    )


def _allowed_hosts() -> frozenset[str] | None:
    raw = os.environ.get("VSH_HTTP_ALLOWED_HOSTS", "").strip()
    if not raw:
        return None
    return frozenset(part.strip().lower() for part in raw.split(",") if part.strip())


def _host_blocked(hostname: str) -> bool:
    allowed = _allowed_hosts()
    lowered = hostname.lower().rstrip(".")
    if allowed is not None:
        return lowered not in allowed
    if lowered == "localhost":
        return True
    try:
        addr = ipaddress.ip_address(lowered)
    except ValueError:
        try:
            infos = socket.getaddrinfo(lowered, None, type=socket.SOCK_STREAM)
        except socket.gaierror:
            return False
        for info in infos:
            ip = info[4][0]
            try:
                parsed = ipaddress.ip_address(ip)
            except ValueError:
                continue
            if any(parsed in network for network in _blocked_networks()):
                return True
        return False
    return any(addr in network for network in _blocked_networks())


def validate_outbound_url(url: str) -> str:
    """Validate URL host against SSRF policy. Returns normalized URL."""
    parsed = urlparse(url.strip())
    hostname = parsed.hostname
    if hostname is None:
        msg = f"URL is missing a host: {url!r}"
        raise SsrfBlockedError(msg)
    if _host_blocked(hostname):
        msg = f"blocked outbound host: {hostname!r}"
        raise SsrfBlockedError(msg)
    return url.strip()


def resolve_allowed_host(url: str) -> str:
    return validate_outbound_url(url)

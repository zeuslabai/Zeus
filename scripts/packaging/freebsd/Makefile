# Zeus Port Makefile for FreeBSD
#
# This port packages the pre-built zeus binary for FreeBSD systems.
# Supports both source-based port build and binary package creation.
#
# Usage:
#   cd /usr/ports/sysutils/zeus && make install
#   Or use bmake + pkg for binary-only install

PORTNAME=       zeus
PORTVERSION=    0.1.0
PORTREVISION=   0
CATEGORIES=     sysutils ai
MAINTAINER=     Zeus Team <team@zeuslab.ai>
COMMENT=       Autonomous AI assistant with 327 tools and 11 LLM providers

# License (MIT OR Apache-2.0)
LICENSE=       MIT

# No source dist — we ship a pre-built binary
DISTFILES=
DIST_SUBDIR=   zeus

# Binary architecture map
.include <bsdpkg.mk>
.if ${ARCH} == "amd64"
  DISTNAME=    zeus-${PORTVERSION}-freebsd-x86_64
.elif ${ARCH} == "aarch64"
  DISTNAME=    zeus-${PORTVERSION}-freebsd-arm64
.else
  DISTNAME=    zeus-${PORTVERSION}-freebsd-${ARCH}
.endif

# ── Build / Install ─────────────────────────────────────────────────────────
do-build:
	@echo "==> Zeus is a pre-built binary — nothing to compile"

do-install:
	@echo "==> Installing zeus to ${PREFIX}/bin/"
	${INSTALL_PROGRAM} ${WRKSRC}/zeus ${PREFIX}/bin/zeus
	${INSTALL_MAN} ${WRKSRC}/zeus.1 ${PREFIX}/man/man1/zeus.1

# ── Wrapper for binary packaging ─────────────────────────────────────────────
# Used by build-port.sh to produce a .txz package without needing the ports tree
dist:
	@echo "Binary-only port — use build-port.sh to create .txz packages"

.include <bsd.port.mk>
PORTNAME=	zeus
DISTVERSION=	0.1.0
CATEGORIES=	sysutils

MAINTAINER=	your@email.com
COMMENT=	Autonomous AI assistant with persistent memory and tool execution
WWW=		https://github.com/zeuslabai/Zeus

LICENSE=	MIT APACHE20
LICENSE_COMB=	dual

BUILD_DEPENDS=	${RUST_DEFAULT}:lang/${RUST_DEFAULT}

USES=		cargo sqlite:3 ssl

USE_GITHUB=	yes
GH_ACCOUNT=	user
GH_PROJECT=	zeus

PLIST_FILES=	bin/zeus \
		"@dir etc/zeus" \
		"@sample etc/zeus/config.toml.sample"

SUB_FILES=	zeus
SUB_LIST=	ZEUS_USER=${ZEUS_USER} \
		ZEUS_GROUP=${ZEUS_GROUP}

USE_RC_SUBR=	zeus

ZEUS_USER?=	zeus
ZEUS_GROUP?=	zeus

USERS=		${ZEUS_USER}
GROUPS=		${ZEUS_GROUP}

do-install:
	${INSTALL_PROGRAM} ${CARGO_TARGET_DIR}/release/zeus \
		${STAGEDIR}${PREFIX}/bin/zeus
	${MKDIR} ${STAGEDIR}${PREFIX}/etc/zeus
	${INSTALL_DATA} ${FILESDIR}/config.toml.sample \
		${STAGEDIR}${PREFIX}/etc/zeus/config.toml.sample

.include <bsd.port.mk>

// -- WP SECQT -- Pairing and key UI (PLAN-SEC 4.3, SEC-C): the Link client
// against hubs of this process's own (dspi_hub_*, on loopback) with TLS
// required, and against the fake hub (plaintext, an upstream hub's stand-in).
// - the controller: the key on the PIN prompt, the v2 credential saved, a v1
//   token pinned and saved at its first TLS use with the new-key notice once,
//   hub_key_changed and tls_refused offering re-pairing (only those codes), a
//   re-pair that asks for the PIN, shows the new key and replaces the
//   credential only when paired;
// - the app's windows: the device line and the device list red with
//   "(unsecured)" and the info button on a plaintext hub, plain on a TLS hub;
//   the info's text is the core's row; the PIN prompt shows the key;
// - Settings > Networking: the hub requires TLS off loopback (startOptions),
//   shows its key with the pairing code, and the upstream compatibility switch
//   asks first with the hub's own list of what it turns off.
// Screenshots in <out>/secqt/.
#include "SmokeDriver.h"

#include "DeviceModel.h"
#include "DevicesController.h"
#include "HubController.h"
#include "LinkController.h"
#include "LinkFakeHub.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"
#include "Snapshot.h"
#include "TokenStore.h"

#include <QColor>
#include <QDir>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <cstdio>
#include <memory>

namespace {
using namespace linksmoke;

QJsonObject parsed(const QByteArray &reply) { return QJsonDocument::fromJson(reply).object(); }

// A hub of this process's own on loopback, serving a demo device of its own
struct OwnHub {
    FfiHub *hub = dspi_hub_new();
    QString id, url, fingerprint, tls;
    QString offUrl;   // -- WP SECFIX --
    int port = 0;
    bool loopbackOnly = false;
    bool compat = false;

    ~OwnHub() {
        stop();
        if (hub) dspi_hub_free(hub);
    }
    bool start(const QString &dir, uint8_t access, uint32_t options) {
        loopbackOnly = options & HUB_OPT_LOOPBACK;
        if (!hub) return false;
        QByteArray buf(int(HUB_REPLY_MIN), '\0');
        const QByteArray d = QDir::toNativeSeparators(dir).toUtf8();
        const uint32_t n = dspi_hub_start(hub, "SECQT Hub", d.constData(), 0, access, options, buf.data(), HUB_REPLY_MIN);
        if (n == 0 || n > HUB_REPLY_MIN) return false;
        const QJsonObject o = parsed(QByteArray(buf.constData(), int(n) - 1));
        if (!o.value(QLatin1String("ok")).toBool()) return false;
        id = o.value(QLatin1String("hubId")).toString();
        port = o.value(QLatin1String("port")).toInt();
        url = QStringLiteral("ws://127.0.0.1:%1/dspi/v1").arg(port);
        // -- WP SECFIX -- the same hub at an address the "local" rule does not
        // cover (it binds every address without HUB_OPT_LOOPBACK)
        offUrl = loopbackOnly ? QString() : QStringLiteral("ws://127.0.0.2:%1/dspi/v1").arg(port);
        fingerprint = o.value(QLatin1String("tlsFingerprint")).toString();
        tls = o.value(QLatin1String("tls")).toString();
        compat = o.value(QLatin1String("upstreamCompat")).toBool();
        return dspi_hub_attach_demo(hub) >= 0;
    }
    QString pin() {
        QByteArray buf(int(HUB_REPLY_MIN), '\0');
        const uint32_t n = dspi_hub_begin_pairing(hub, buf.data(), HUB_REPLY_MIN);
        if (n == 0 || n > HUB_REPLY_MIN) return QString();
        return parsed(QByteArray(buf.constData(), int(n) - 1)).value(QLatin1String("pin")).toString();
    }
    void stop() {
        if (hub) dspi_hub_stop(hub);
    }
};

// A core and a controller with a token file of their own: one run of an app
struct Run {
    FfiCore *core;
    std::unique_ptr<LinkController> link;
    explicit Run(const QString &file) : core(dspi_core_new()) {
        link = std::make_unique<LinkController>(core, std::make_unique<TokenStore>(nullptr, file), nullptr);
    }
    ~Run() {
        link.reset();
        dspi_core_free(core);
    }
};

QString storedIn(const QString &file, const QString &hub) { return TokenStore(nullptr, file).load(hub); }

// The key a stored credential is pinned to, "" for a v1 one
QString pinnedKey(const QString &credential) {
    const QByteArray c = credential.toUtf8();
    return QString::fromUtf8(readSized([&](char *b, uint32_t n) { return dspi_link_credential_hub_key(c.constData(), b, n); }));
}

// -- WP PINTLS -- the record reaches its hub over TLS only (pinned, or pinned before)
bool tlsOnly(const QString &credential) { return dspi_link_credential_tls_only(credential.toUtf8().constData()); }

bool click(QQuickItem *item) {
    return item && item->isVisible() && item->isEnabled() && QMetaObject::invokeMethod(item, "clicked");
}

// ---- The controller ---------------------------------------------------------------

void controllerChecks(SmokeDriver &d, const QString &scratch) {
    const auto settled = [&d](LinkController &l) { return d.waitFor(30000, [&] { return !l.busy(); }); };
    const auto phaseIs = [&d](LinkController &l, const char *phase) {
        return d.waitFor(5000, [&] { return l.phase() == QLatin1String(phase); });
    };

    // Re-pairing is offered for exactly these codes
    const QStringList offered{ "hub_key_changed", "hub_key_unverified", "tls_refused", "tls_required",
                               "tls_required_pinned" };   // -- WP PINTLS -- its offer is Forget Hub
    bool rule = true;
    for (const QString &code : offered) rule = rule && LinkController::offersRePair(code);
    for (const char *code : { "wrong_pin", "wrong_hub", "io", "timeout", "rate_limited", "handshake_refused", "" })
        rule = rule && !LinkController::offersRePair(QString::fromLatin1(code));
    d.check("secqt: re-pairing only for hub_key_changed, hub_key_unverified, tls_refused, tls_required (+ pintls: "
            "tls_required_pinned)", rule);

    // The core's words for an unencrypted session
    {
        Run run(scratch + QStringLiteral("/words.json"));
        const QJsonObject row = parsed(readSized([](char *b, uint32_t n) { return dspi_notice_for("dlink.unsecured", nullptr, b, n); }));
        d.check("secqt: the label is the core's row dlink.unsecured",
                row.value(QLatin1String("key")) == QLatin1String("dlink.unsecured")
                    && run.link->unsecuredLabel() == row.value(QLatin1String("title")).toString()
                    && run.link->unsecuredLabel() == QLatin1String("unsecured"));
        d.check("secqt: the info is the core's row dlink.unsecured.info, calm (trusted network, VPN)",
                !run.link->unsecuredInfoTitle().isEmpty() && run.link->unsecuredInfoText().contains(QLatin1String("VPN"))
                    && run.link->unsecuredInfoText().contains(QLatin1String("trusted local network")));
        d.check("secqt: nothing unsecured without a session", !run.link->unsecured() && !run.link->encrypted());
    }

    // ---- A TLS hub: the key on the prompt, the v2 credential --------------------------
    const QString file = scratch + QStringLiteral("/tokens.json");
    const QString store = scratch + QStringLiteral("/hub-tls");
    auto tlsHub = std::make_unique<OwnHub>();
    const bool up = tlsHub->start(store, HUB_ACCESS_PIN, HUB_OPT_LOOPBACK | HUB_OPT_TLS_REQUIRE);
    d.check("secqt: a hub with TLS required runs, with a key", up && tlsHub->tls == QLatin1String("require")
                                                                 && tlsHub->fingerprint.startsWith(QLatin1String("SHA256:")));
    if (!up) return;
    const QString firstKey = tlsHub->fingerprint;
    {
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(tlsHub->url, tlsHub->id);
        d.check("secqt: TLS hub: asks for the PIN", settled(l) && phaseIs(l, "needs_pairing") && l.errorCode().isEmpty());
        d.check("secqt: TLS hub: encrypted before the PIN, the hub's key shown",
                l.encrypted() && !l.unsecured() && l.hubKey() == firstKey);
        const QString pin = tlsHub->pin();
        l.pair(pin);
        d.check("secqt: TLS hub: paired", settled(l) && l.errorCode().isEmpty() && phaseIs(l, "choose_device"));
        d.check("secqt: TLS hub: a v2 credential saved, pinned to the shown key", d.waitFor(3000, [&] {
            const QString c = storedIn(file, tlsHub->id);
            return !l.busy() && c.startsWith(QLatin1String("v2.")) && pinnedKey(c) == firstKey;
        }));
        d.check("secqt: TLS hub: no key notice after a pairing that showed the key", l.keyNotice().isEmpty());
        d.check("secqt: TLS hub: still encrypted once paired", l.encrypted() && !l.unsecured());
    }
    const QString v2 = storedIn(file, tlsHub->id);
    {
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(tlsHub->url, tlsHub->id);
        d.check("secqt: TLS hub again: the pinned credential works, no PIN",
                settled(l) && l.errorCode().isEmpty() && phaseIs(l, "choose_device") && l.encrypted());
        d.check("secqt: TLS hub again: nothing new to save, no notice",
                d.waitFor(1000, [&] { return !l.busy(); }) && storedIn(file, tlsHub->id) == v2 && l.keyNotice().isEmpty());
    }

    // ---- -- WP SECFIX -- dspi://pair links, through the core's parser ------------------
    {
        const QString linkFile = scratch + QStringLiteral("/link-tokens.json");
        Run run(linkFile);
        LinkController &l = *run.link;
        d.check("secfix: not a pairing link: refused, the link not repeated",
                !l.openPairLink(QStringLiteral("dspi://pair?h=x&p=135790")) && l.errorCode() == QLatin1String("invalid")
                    && !l.errorText().contains(QLatin1String("135790")) && !l.errorText().contains(QLatin1String("dspi://")));
        // The key is the v2 credential's third part (base64url, as k= takes it)
        const QString key = v2.section(QLatin1Char('.'), 2, 2);
        const QString authority = QStringLiteral("127.0.0.1:%1").arg(tlsHub->port);
        const QString wrongKey(43, QLatin1Char('A'));
        const QString wrong = QStringLiteral("dspi://pair?h=%1&a=%2&k=%3&p=246801").arg(tlsHub->id, authority, wrongKey);
        d.check("secfix: a link with a wrong k= (pasted as an address) is followed",
                key.length() == 43 && l.connectToAddress(wrong));
        const bool failed = settled(l);
        d.check("secfix: wrong k=: refused before any PIN, with no re-pairing offered",
                failed && l.errorCode().startsWith(QLatin1String("hub_key")) && !l.rePairOffered()
                    && phaseIs(l, "idle") && storedIn(linkFile, tlsHub->id).isEmpty()
                    && !l.errorText().contains(QLatin1String("246801")));
        if (!l.errorCode().startsWith(QLatin1String("hub_key")))
            std::fprintf(stderr, "secfix: wrong k= gave code %s, phase %s\n", qPrintable(l.errorCode()), qPrintable(l.phase()));
        d.check("secfix: wrong k=: the link's PIN is dropped", l.pairLinkPin().isEmpty());
        const QString pin = tlsHub->pin();
        const QString right = QStringLiteral("dspi://pair?h=%1&a=%2&k=%3&p=%4").arg(tlsHub->id, authority, key, pin);
        d.check("secfix: the hub's own link is followed", l.openPairLink(right));
        d.check("secfix: the PIN prompt, with the hub's key and the link's PIN filled in (no pairing yet)",
                settled(l) && phaseIs(l, "needs_pairing") && l.errorCode().isEmpty() && l.encrypted()
                    && l.hubKey() == firstKey && l.pairLinkPin() == pin && storedIn(linkFile, tlsHub->id).isEmpty());
        l.pair(l.pairLinkPin());
        d.check("secfix: paired on the tap: a v2 credential pinned to the link's key, the PIN spent",
                settled(l) && l.errorCode().isEmpty() && phaseIs(l, "choose_device") && l.pairLinkPin().isEmpty()
                    && d.waitFor(3000, [&] { return pinnedKey(storedIn(linkFile, tlsHub->id)) == firstKey; }));
        l.closeSession();
        settled(l);
    }

    // ---- The hub's key changes: re-pair only ------------------------------------------
    tlsHub->stop();
    QFile::remove(store + QStringLiteral("/tls/key.pem"));
    tlsHub = std::make_unique<OwnHub>();
    const bool again = tlsHub->start(store, HUB_ACCESS_PIN, HUB_OPT_LOOPBACK | HUB_OPT_TLS_REQUIRE);
    d.check("secqt: the same hub with a new key", again && !tlsHub->fingerprint.isEmpty() && tlsHub->fingerprint != firstKey);
    if (!again) return;
    {
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(tlsHub->url, tlsHub->id);
        d.check("secqt: key changed: hub_key_changed, re-pairing offered",
                settled(l) && l.errorCode() == QLatin1String("hub_key_changed") && l.rePairOffered() && !l.errorText().isEmpty());
        d.check("secqt: key changed: no session, the credential kept", phaseIs(l, "idle") && storedIn(file, tlsHub->id) == v2);
        l.rePair();
        d.check("secqt: re-pair: the hub asks for its PIN, showing its new key",
                settled(l) && phaseIs(l, "needs_pairing") && l.errorCode().isEmpty() && l.encrypted()
                    && l.hubKey() == tlsHub->fingerprint);
        // -- WP PINTLS -- the token goes, the hub stays TLS-only until paired
        d.check("pintls: re-pair: the token dropped, the record TLS-only with no key until paired",
                tlsOnly(storedIn(file, tlsHub->id)) && pinnedKey(storedIn(file, tlsHub->id)).isEmpty()
                    && storedIn(file, tlsHub->id) != v2);
        l.pair(tlsHub->pin());
        d.check("secqt: re-pair: paired", settled(l) && l.errorCode().isEmpty() && phaseIs(l, "choose_device"));
        d.check("secqt: re-pair: the credential now pins the new key", d.waitFor(3000, [&] {
            const QString c = storedIn(file, tlsHub->id);
            return !l.busy() && c != v2 && pinnedKey(c) == tlsHub->fingerprint;
        }));
        l.dismissRePair();
        d.check("secqt: dismissing with no failure changes nothing", l.errorCode().isEmpty());
    }
    tlsHub.reset();

    // ---- A v1 token, then TLS: pinned once, the notice once -------------------------------
    const QString v1Store = scratch + QStringLiteral("/hub-v1");
    {
        OwnHub plain;
        // -- WP SECFIX -- on every address, dialled at 127.0.0.2: not local
        const bool plainUp = plain.start(v1Store, HUB_ACCESS_PIN, 0);
        d.check("secqt: a hub without TLS runs", plainUp && plain.tls == QLatin1String("off") && plain.fingerprint.isEmpty());
        if (!plainUp) return;
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(plain.offUrl, plain.id);
        d.check("secqt: no TLS: asks for the PIN, unencrypted", settled(l) && phaseIs(l, "needs_pairing")
                                                                     && d.waitFor(3000, [&] { return l.unsecured(); })
                                                                     && !l.encrypted() && l.hubKey().isEmpty());
        l.pair(plain.pin());
        d.check("secqt: no TLS: paired, a v1 token saved", settled(l) && l.errorCode().isEmpty() && d.waitFor(3000, [&] {
            const QString c = storedIn(file, plain.id);
            return !l.busy() && !c.isEmpty() && !c.startsWith(QLatin1String("v2.")) && pinnedKey(c).isEmpty();
        }));
        d.check("secqt: no TLS: unsecured while in session", l.unsecured());
        l.closeSession();
        d.check("secqt: no TLS: nothing unsecured once left", settled(l) && phaseIs(l, "idle") && !l.unsecured());
        // -- WP SECFIX -- the same hub on 127.0.0.1: local, so not unsecured
        l.connectToHub(plain.url, plain.id);
        d.check("secfix: no TLS on loopback: local, not unsecured",
                settled(l) && l.errorCode().isEmpty() && phaseIs(l, "choose_device")
                    && d.waitFor(3000, [&] { return l.local(); }) && !l.unsecured() && !l.encrypted());
        l.closeSession();
        d.check("secfix: left: not local any more", settled(l) && phaseIs(l, "idle") && !l.local());
    }
    {
        OwnHub upgraded;
        const bool upgradedUp = upgraded.start(v1Store, HUB_ACCESS_PIN, HUB_OPT_LOOPBACK | HUB_OPT_TLS_REQUIRE);
        d.check("secqt: the same hub, now with TLS required", upgradedUp && !upgraded.fingerprint.isEmpty());
        if (!upgradedUp) return;
        const QString v1 = storedIn(file, upgraded.id);
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(upgraded.url, upgraded.id);
        d.check("secqt: v1 upgrade: the old token works over TLS", settled(l) && l.errorCode().isEmpty()
                                                                      && phaseIs(l, "choose_device") && l.encrypted());
        d.check("secqt: v1 upgrade: saved as a v2 credential pinned to the hub's key", d.waitFor(3000, [&] {
            const QString c = storedIn(file, upgraded.id);
            return !l.busy() && c != v1 && c.startsWith(QLatin1String("v2.")) && pinnedKey(c) == upgraded.fingerprint;
        }));
        d.check("secqt: v1 upgrade: the new-key notice names the key",
                l.keyNotice().contains(upgraded.fingerprint) && l.keyNotice().startsWith(QLatin1String("Encrypted connection set up")));
        l.dismissKeyNotice();
        d.check("secqt: v1 upgrade: the notice can be dismissed", l.keyNotice().isEmpty());
        l.refreshStatus();
        d.spin(500);
        d.check("secqt: v1 upgrade: the notice comes once", l.keyNotice().isEmpty());
    }
    {
        // ... and the next run of the app neither asks nor says it again
        OwnHub upgraded;
        upgraded.start(v1Store, HUB_ACCESS_PIN, HUB_OPT_LOOPBACK | HUB_OPT_TLS_REQUIRE);
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(upgraded.url, upgraded.id);
        d.check("secqt: v1 upgrade: next run, pinned, no notice", settled(l) && l.errorCode().isEmpty()
                                                                     && phaseIs(l, "choose_device") && l.keyNotice().isEmpty());
    }

    // ---- -- WP PINTLS -- a hub once pinned stays TLS-only after Pair Again ------------------
    // (owner decision 2026-11-17) The pinned hub comes back without TLS: Pair Again
    // never pairs it in plaintext; only Forget Hub does, and the bar is then red.
    {
        OwnHub off;
        const bool offUp = off.start(v1Store, HUB_ACCESS_PIN, 0);
        d.check("pintls: the pinned hub, now without TLS", offUp && off.tls == QLatin1String("off"));
        if (!offUp) return;
        const QString pinned = storedIn(file, off.id);
        d.check("pintls: its record is pinned", !pinnedKey(pinned).isEmpty());
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(off.offUrl, off.id);
        d.check("pintls: pinned, no TLS: tls_refused, Pair Again offered, no session",
                settled(l) && l.errorCode() == QLatin1String("tls_refused") && l.rePairOffered() && phaseIs(l, "idle"));
        l.rePair();
        d.check("pintls: Pair Again: tls_required_pinned before any PIN, no session",
                settled(l) && l.errorCode() == QLatin1String("tls_required_pinned") && l.rePairOffered()
                    && phaseIs(l, "idle") && !l.unsecured());
        d.check("pintls: Pair Again: the record still TLS-only, the token gone", d.waitFor(3000, [&] {
            const QString c = storedIn(file, off.id);
            return !l.busy() && tlsOnly(c) && pinnedKey(c).isEmpty() && c != pinned;
        }));
        d.check("pintls: the words say Forget Hub is the way to no encryption",
                l.errorText().contains(QLatin1String("forget the hub")));
        l.rePair();   // the offer is Forget Hub now
        d.check("pintls: Forget Hub: the hub asks for its PIN, unencrypted and red",
                settled(l) && phaseIs(l, "needs_pairing") && l.errorCode().isEmpty() && !l.encrypted()
                    && d.waitFor(3000, [&] { return l.unsecured(); }));
        d.check("pintls: Forget Hub: the record is gone", storedIn(file, off.id).isEmpty());
        l.pair(off.pin());
        d.check("pintls: Forget Hub: paired in plaintext, a v1 token", settled(l) && d.waitFor(3000, [&] {
            const QString c = storedIn(file, off.id);
            return !l.busy() && !c.isEmpty() && !tlsOnly(c);
        }));
        l.closeSession();
        settled(l);
    }

    // ---- Advertised TLS never falls back: tls_refused ----------------------------------------
    FakeHubProcess plain;
    const bool fakeUp = plain.start({ "--auth", "none" }, QStringLiteral("127.0.0.2:0"));   // -- WP SECFIX -- not local
    d.check("secqt: the fake hub runs (DSPI_SMOKE_FAKEHUB)", fakeUp);
    if (!fakeUp) return;
    {
        Run run(file);
        LinkController &l = *run.link;
        l.connectToHub(plain.url, plain.id, true);
        d.check("secqt: tls advertised, refused: tls_refused, re-pairing offered, no session",
                settled(l) && l.errorCode() == QLatin1String("tls_refused") && l.rePairOffered() && phaseIs(l, "idle"));
        l.dismissRePair();
        d.check("secqt: dismissed", l.errorCode().isEmpty() && !l.rePairOffered());
        l.connectToHub(plain.url, plain.id, false);
        d.check("secqt: not advertised: the plaintext hub connects, unsecured",
                settled(l) && l.errorCode().isEmpty() && d.waitFor(3000, [&] { return l.unsecured(); }) && !l.encrypted());
        d.check("secqt: a wrong PIN offers no re-pairing", !LinkController::offersRePair(QStringLiteral("wrong_pin")));
    }
}

// ---- The app's windows -------------------------------------------------------------------

void windowChecks(SmokeDriver &d, const QString &scratch) {
    auto *link = Registrar::instance<LinkController>();
    auto *list = Registrar::instance<DevicesController>();
    DeviceModel *device = d.app().device;
    QQuickItem *host = list ? list->host() : nullptr;
    QObject *popup = host ? host->findChild<QObject *>(QStringLiteral("DevicesPopup")) : nullptr;
    QObject *prompt = host ? host->findChild<QObject *>(QStringLiteral("PairingDialog")) : nullptr;
    QObject *rePairDialog = host ? host->findChild<QObject *>(QStringLiteral("RePairDialog")) : nullptr;
    d.check("secqt: the controllers, the list, the prompt and the re-pair question are loaded",
            link && list && popup && prompt && rePairDialog);
    if (!link || !list || !popup || !prompt || !rePairDialog) return;
    QQuickWindow *main = d.mainWindow();
    const auto grab = [&](const char *name) { d.grab(main, QStringLiteral("secqt/%1.png").arg(QLatin1String(name))); };
    const auto settled = [&] { return d.waitFor(30000, [link] { return !link->busy(); }); };
    const auto in = [&](QObject *p, const QString &name) {
        auto *content = p->property("contentItem").value<QQuickItem *>();
        QQuickItem *scope = content && content->parentItem() ? content->parentItem() : content;
        return scope ? d.item(name, scope) : nullptr;
    };
    const auto opened = [](QObject *p) { return p && p->property("opened").toBool(); };
    const auto bindFirst = [&] {
        if (!d.waitFor(15000, [&] { return link->phase() == QLatin1String("choose_device") && !link->devices().isEmpty(); }))
            return false;
        const QVariantMap m = link->devices().first().toMap();
        list->chooseHubDevice(m.value(QStringLiteral("handle")).toInt(), m.value(QStringLiteral("serial")).toString());
        return settled() && d.waitFor(15000, [&] { return link->phase() == QLatin1String("bound") && device->remote() && device->connected(); })
               && d.waitIdle(20000);
    };
    const auto leave = [&] {
        link->closeSession();
        settled();
        d.waitFor(10000, [&] { return !device->remote() && device->connected(); });
        d.waitIdle();
    };
    QQuickItem *statusButton = d.item(QStringLiteral("DeviceStatus.button"));
    QQuickItem *statusText = d.item(QStringLiteral("DeviceStatus.text"));
    QQuickItem *statusLabel = d.item(QStringLiteral("DeviceStatus.unsecured"));
    QQuickItem *statusInfo = d.item(QStringLiteral("DeviceStatus.unsecuredInfo"));
    d.check("secqt: the device line has its unsecured label and info button",
            statusButton && statusText && statusLabel && statusInfo);
    if (!statusButton || !statusText || !statusLabel || !statusInfo) return;
    const auto background = [&] {
        auto *bg = statusButton->property("background").value<QQuickItem *>();
        return bg ? bg->property("color").value<QColor>() : QColor();
    };
    const QColor plainBackground = background();
    d.check("secqt: on the demo, the device line is not unsecured",
            device->demo() && !statusLabel->isVisible() && !statusInfo->isVisible());

    // ---- A plaintext hub: red, "(unsecured)", the info -------------------------------------
    {
        FakeHubProcess hub;
        const bool up = hub.start({ "--auth", "none" }, QStringLiteral("127.0.0.2:0"));   // -- WP SECFIX -- not local
        d.check("secqt: the fake hub runs (DSPI_SMOKE_FAKEHUB)", up);
        if (!up) return;
        link->connectToHub(hub.url, hub.id);
        d.check("secqt: plaintext hub: bound", settled() && bindFirst());
        d.check("secqt: plaintext hub: the device line says (unsecured), in red", d.waitFor(3000, [&] {
            return statusLabel->isVisible() && statusLabel->property("text").toString() == QLatin1String("(unsecured)")
                   && statusInfo->isVisible();
        }));
        d.check("secqt: plaintext hub: its background is red, not the plain one",
                background() != plainBackground && background().red() > background().blue());
        d.check("secqt: plaintext hub: its accessible name says so",
                statusButton->property("shownLine").toString().endsWith(QLatin1String(" (unsecured)")));
        // The info button opens the core's explanation
        QObject *info = statusInfo->property("dialog").value<QObject *>();
        d.check("secqt: the info button opens the explanation", click(statusInfo) && d.waitFor(2000, [&] { return opened(info); }));
        d.check("secqt: the explanation is the core's row",
                info && info->property("title").toString() == link->unsecuredInfoTitle()
                    && info->property("text").toString() == link->unsecuredInfoText());
        grab("unsecured-info");
        if (info) QMetaObject::invokeMethod(info, "close");
        d.check("secqt: and closes", d.waitFor(2000, [&] { return !opened(info); }));
        // The device list says it too
        d.app().shell->trigger(QStringLiteral("device.devices"));
        d.check("secqt: the device list opens", d.waitFor(2000, [&] { return opened(popup); }));
        QQuickItem *listLabel = in(popup, QStringLiteral("DevicesPopup.unsecured"));
        QQuickItem *listInfo = in(popup, QStringLiteral("DevicesPopup.unsecuredInfo"));
        d.check("secqt: the device list's session line says (unsecured), with the info button",
                listLabel && listLabel->isVisible() && listLabel->property("text").toString() == QLatin1String("(unsecured)")
                    && listInfo && listInfo->isVisible());
        d.check("secqt: no key line on an unencrypted session",
                in(popup, QStringLiteral("DevicesPopup.hubKey")) && !in(popup, QStringLiteral("DevicesPopup.hubKey"))->isVisible());
        grab("unsecured-list");
        QMetaObject::invokeMethod(popup, "close");
        d.waitFor(2000, [&] { return !opened(popup); });
        leave();
        d.check("secqt: left: the line is plain again", d.waitFor(3000, [&] { return !statusLabel->isVisible() && !statusInfo->isVisible(); })
                                                         && background() == plainBackground);
    }

    // ---- A TLS hub: the key on the prompt, then plain -----------------------------------------
    {
        OwnHub hub;
        const bool up = hub.start(scratch + QStringLiteral("/hub-window"), HUB_ACCESS_PIN, HUB_OPT_LOOPBACK | HUB_OPT_TLS_REQUIRE);
        d.check("secqt: a TLS hub for the windows", up);
        if (!up) return;
        link->connectToHub(hub.url, hub.id);
        d.check("secqt: TLS hub: the prompt opens", settled() && d.waitFor(5000, [&] { return opened(prompt); }));
        QQuickItem *key = in(prompt, QStringLiteral("PairingDialog.hubKey"));
        d.check("secqt: TLS hub: the prompt shows the hub's key before the PIN",
                key && key->isVisible() && key->property("text").toString() == hub.fingerprint);
        d.check("secqt: TLS hub: and no unencrypted line", in(prompt, QStringLiteral("PairingDialog.unsecured"))
                                                              && !in(prompt, QStringLiteral("PairingDialog.unsecured"))->isVisible());
        grab("pairing-key");
        link->pair(hub.pin());
        d.check("secqt: TLS hub: paired", settled() && d.waitFor(5000, [&] { return !opened(prompt); }) && link->errorCode().isEmpty());
        d.check("secqt: TLS hub: bound", bindFirst());
        d.spin(300);
        d.check("secqt: TLS hub: the device line is not unsecured",
                link->encrypted() && !statusLabel->isVisible() && !statusInfo->isVisible() && background() == plainBackground
                    && !statusButton->property("shownLine").toString().contains(QLatin1String("unsecured")));
        d.app().shell->trigger(QStringLiteral("device.devices"));
        d.check("secqt: TLS hub: the list shows the verified key", d.waitFor(2000, [&] { return opened(popup); }) && d.waitFor(2000, [&] {
            QQuickItem *k = in(popup, QStringLiteral("DevicesPopup.hubKey"));
            QQuickItem *u = in(popup, QStringLiteral("DevicesPopup.unsecured"));
            return k && k->isVisible() && k->property("text").toString().contains(hub.fingerprint) && u && !u->isVisible();
        }));
        grab("encrypted-list");
        QMetaObject::invokeMethod(popup, "close");
        d.waitFor(2000, [&] { return !opened(popup); });
        leave();
        hub.stop();

        // The key changes: the question offers pairing again, and Cancel leaves it
        QFile::remove(scratch + QStringLiteral("/hub-window/tls/key.pem"));
        OwnHub changed;
        const bool changedUp = changed.start(scratch + QStringLiteral("/hub-window"), HUB_ACCESS_PIN,
                                             HUB_OPT_LOOPBACK | HUB_OPT_TLS_REQUIRE);
        d.check("secqt: the window's hub with a new key", changedUp && changed.fingerprint != hub.fingerprint);
        if (changedUp) {
            link->connectToHub(changed.url, changed.id);
            d.check("secqt: hub_key_changed opens the re-pair question",
                    settled() && link->errorCode() == QLatin1String("hub_key_changed")
                        && d.waitFor(3000, [&] { return opened(rePairDialog); }));
            d.check("secqt: it is titled for the key, with Pair Again and Cancel only",
                    rePairDialog->property("title").toString() == QLatin1String("This hub's key changed")
                        && rePairDialog->property("acceptText").toString() == QLatin1String("Pair Again")
                        && rePairDialog->property("rejectText").toString() == QLatin1String("Cancel")
                        && rePairDialog->property("alternativeText").toString().isEmpty());
            grab("re-pair");
            QMetaObject::invokeMethod(rePairDialog, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
            d.check("secqt: Cancel leaves it, with no session", d.waitFor(3000, [&] { return !opened(rePairDialog); })
                                                                    && link->errorCode().isEmpty() && link->phase() == QLatin1String("idle"));
            link->connectToHub(changed.url, changed.id);
            d.check("secqt: again: the question", settled() && d.waitFor(3000, [&] { return opened(rePairDialog); }));
            QMetaObject::invokeMethod(rePairDialog, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
            d.check("secqt: Pair Again: the prompt with the new key", settled() && d.waitFor(5000, [&] { return opened(prompt); })
                                                                         && link->hubKey() == changed.fingerprint);
            QMetaObject::invokeMethod(prompt, "reject");
            d.check("secqt: the prompt's Cancel leaves the hub", settled() && d.waitFor(5000, [&] {
                return !opened(prompt) && link->phase() == QLatin1String("idle");
            }));
        }
    }
    d.check("secqt: the demo is back", d.waitFor(10000, [&] { return device->demo() && device->connected() && !device->remote(); }));
}

// ---- Settings > Networking -----------------------------------------------------------------

void networkingChecks(SmokeDriver &d, const QString &scratch) {
    // The TLS mode a start asks for
    const uint32_t app = HUB_OPT_USB | HUB_OPT_APP;
    const uint32_t tlsBits = HUB_OPT_TLS_ON | HUB_OPT_TLS_REQUIRE | HUB_OPT_UPSTREAM_COMPAT;
    d.check("secqt: on the network the hub requires TLS by default",
            HubController::startOptions(0, false) == (app | HUB_OPT_TLS_REQUIRE));
    d.check("secqt: on loopback TLS is off", (HubController::startOptions(HUB_OPT_LOOPBACK, false) & tlsBits) == 0);
    d.check("secqt: compatibility mode replaces require",
            HubController::startOptions(0, true) == (app | HUB_OPT_UPSTREAM_COMPAT)
                && HubController::startOptions(HUB_OPT_LOOPBACK | HUB_OPT_TLS_REQUIRE, true)
                       == (app | HUB_OPT_LOOPBACK | HUB_OPT_UPSTREAM_COMPAT));
    d.check("secqt: a mode named by the caller is kept",
            HubController::startOptions(HUB_OPT_LOOPBACK | HUB_OPT_TLS_REQUIRE, false) == (app | HUB_OPT_LOOPBACK | HUB_OPT_TLS_REQUIRE));

    auto *hub = Registrar::instance<HubController>();
    auto *settings = Registrar::instance<SettingsController>();
    d.check("secqt: the hub controller and settings exist", hub && settings);
    if (!hub || !settings) return;
    const QStringList effects = hub->compatEffects();
    d.check("secqt: the compatibility effects are the hub's list", effects.size() >= 5
                                                                   && effects.join(QLatin1Char(' ')).contains(QLatin1String("TLS is no longer required")));
    d.check("secqt: compatibility mode is off by default", !hub->upstreamCompat());

    hub->setStoreDir(scratch + QStringLiteral("/own-hub"));
    hub->setExtraOptions(HUB_OPT_LOOPBACK | HUB_OPT_TLS_REQUIRE);
    d.app().shell->trigger(QStringLiteral("edit.settings"));
    auto *w = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("edit.settings")));
    d.check("secqt: the Settings window opens", w && d.waitFor(3000, [&]() { return w->isVisible(); }));
    if (!w) return;
    settings->open(QStringLiteral("networking"));
    d.spin(400);
    QQuickItem *scope = w->contentItem();
    const auto item = [&](const char *name) { return d.item(QString::fromLatin1(name), scope); };
    hub->setSharing(true);
    d.check("secqt: sharing with TLS required", d.waitFor(15000, [&] { return hub->running() && !hub->busy(); })
                                                 && hub->tlsMode() == QLatin1String("require")
                                                 && hub->tlsFingerprint().startsWith(QLatin1String("SHA256:")));
    d.spin(300);
    d.check("secqt: the page shows the hub's key", item("Networking.tlsKey") && item("Networking.tlsKey")->isVisible()
                                                     && item("Networking.tlsKey")->property("text") == hub->tlsFingerprint());
    hub->allowNewClient();
    d.check("secqt: the key shows with the pairing code", d.waitFor(5000, [&] { return hub->pairingCode().size() == 6; })
                                                            && d.waitFor(1000, [&] {
                                                                   QQuickItem *k = item("Networking.pairingKey");
                                                                   return k && k->isVisible() && k->property("text").toString().contains(hub->tlsFingerprint());
                                                               }));
    d.grab(w, QStringLiteral("secqt/networking-key.png"));
    hub->cancelPairing();

    // A client pairs over TLS with the page's code and pins the page's key
    {
        const QString pinFile = scratch + QStringLiteral("/own-hub-tokens.json");
        Run run(pinFile);
        LinkController &l = *run.link;
        l.connectToAddress(hub->address());
        d.check("secqt: a client sees the page's key on its prompt", d.waitFor(30000, [&] { return !l.busy(); })
                                                                     && d.waitFor(5000, [&] { return l.phase() == QLatin1String("needs_pairing"); })
                                                                     && l.hubKey() == hub->tlsFingerprint());
        hub->allowNewClient();
        d.waitFor(5000, [&] { return hub->pairingCode().size() == 6; });
        l.pair(hub->pairingCode());
        d.check("secqt: and pins it", d.waitFor(30000, [&] { return !l.busy(); }) && d.waitFor(3000, [&] {
            return pinnedKey(storedIn(pinFile, hub->hubId())) == hub->tlsFingerprint();
        }));
    }

    // The compatibility switch: asks first, with the hub's list
    QQuickItem *toggle = item("Networking.upstreamCompat");
    QQuickItem *warning = item("Networking.compatWarning");
    QQuickItem *page = item("NetworkingPage");
    QObject *question = page ? page->property("compatDialog").value<QObject *>() : nullptr;
    if (!toggle || !warning || !question)
        std::fprintf(stderr, "secqt: toggle %p warning %p question %p\n", (void *)toggle, (void *)warning, (void *)question);
    d.check("secqt: the switch, its warning and its question exist", toggle && warning && question);
    if (!toggle || !warning || !question) return;
    d.check("secqt: no warning while off", !warning->isVisible() && !toggle->property("checked").toBool());
    QMetaObject::invokeMethod(toggle, "toggled", Q_ARG(bool, true));
    d.check("secqt: switching it on asks first", d.waitFor(2000, [&] { return question->property("opened").toBool(); })
                                                  && !hub->upstreamCompat());
    const QString text = question->property("text").toString();
    bool all = true;
    for (const QString &e : effects) all = all && text.contains(e);
    d.check("secqt: the question lists every effect, as the hub words them", all && text.contains(QLatin1String("downgrades")));
    d.grab(w, QStringLiteral("secqt/compat-question.png"));
    QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
    d.check("secqt: Cancel leaves it off", d.waitFor(2000, [&] { return !question->property("opened").toBool(); })
                                            && !hub->upstreamCompat() && !toggle->property("checked").toBool());
    QMetaObject::invokeMethod(toggle, "toggled", Q_ARG(bool, true));
    d.waitFor(2000, [&] { return question->property("opened").toBool(); });
    QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
    d.check("secqt: Turn On restarts the hub in compatibility mode",
            d.waitFor(20000, [&] { return hub->upstreamCompat() && hub->running() && !hub->busy() && hub->tlsMode() == QLatin1String("on"); }));
    d.check("secqt: stored as hub/upstreamCompat", QSettings().value(QStringLiteral("hub/upstreamCompat")).toBool());
    d.spin(300);
    d.check("secqt: the warning stays on the page, every effect listed",
            warning->isVisible() && toggle->property("checked").toBool()
                && item("Networking.compatWarningTitle")->property("text").toString().startsWith(QLatin1String("Security downgraded")));
    d.grab(w, QStringLiteral("secqt/compat-on.png"));
    QMetaObject::invokeMethod(toggle, "toggled", Q_ARG(bool, false));
    d.check("secqt: off again: TLS required again, no warning",
            d.waitFor(20000, [&] { return !hub->upstreamCompat() && hub->running() && !hub->busy() && hub->tlsMode() == QLatin1String("require"); })
                && d.waitFor(1000, [&] { return !warning->isVisible(); }));

    hub->setSharing(false);
    d.waitFor(15000, [&] { return !hub->running() && !hub->busy(); });
    hub->setExtraOptions(0);
    w->close();
    QSettings().remove(QStringLiteral("hub"));
}

void securitySection(SmokeDriver &d) {
    const QString scratch = d.outDir() + QStringLiteral("/secqt");
    QDir(scratch).removeRecursively();
    QDir().mkpath(scratch);
    controllerChecks(d, scratch);
    windowChecks(d, scratch);
    networkingChecks(d, scratch);
}

} // namespace

static const SmokeDriver::Section security("secqt-security", 133, &securitySection);

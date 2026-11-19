// -- WP PGQT -- The open device's protocol, the raw request dialog by id and
// the alink sender's page (DeviceInfoController.h)
#include "DeviceInfoController.h"

#include "Core.h"
#include "DeviceModel.h"
#include "Shell.h"
#include "Snapshot.h"

#include "CoreApi.h"

#include <QCoreApplication>
#include <QJSEngine>
#include <QJsonArray>
#include <QJsonDocument>
#include <QRegularExpression>
#include <memory>

static const Registrar::Controller<DeviceInfoController> deviceInfoController;
static const Registrar::Banner senderBanner("SenderBanner", 9);   // -- WP QTSENDER --

namespace {

QJsonObject parse(const QByteArray &json) { return QJsonDocument::fromJson(json).object(); }

// The pages of the registry this build vendors (dspi-proto's paged registry,
// upstream dspi_commands.json at the pinned commit), and our extension page
QString pageName(int page) {
    switch (page) {
    case 0x01: return QStringLiteral("SYS");
    case 0x02: return QStringLiteral("STORE");
    case 0x03: return QStringLiteral("INPUT");
    case 0x04: return QStringLiteral("OUTPUT");
    case 0x05: return QStringLiteral("ROUTE");
    case 0x06: return QStringLiteral("FILTER");
    case 0x07: return QStringLiteral("DYN");
    case 0x08: return QStringLiteral("FX");
    case 0x09: return QStringLiteral("MEAS");
    case 0x0A: return QStringLiteral("CS");
    case 0x0B: return QStringLiteral("XPORT");
    case 0x0C: return QStringLiteral("CLOCK");
    case 0xE0: return QStringLiteral("EXT");
    case 0xF0: return QStringLiteral("DIAG");
    default: return QString();
    }
}

// The firmware's result codes by name (vc_results of the same registry)
QString resultName(int code) {
    switch (code) {
    case 0x00: return QStringLiteral("VC_OK");
    case 0x10: return QStringLiteral("VC_ERR_UNKNOWN");
    case 0x11: return QStringLiteral("VC_ERR_DIRECTION");
    case 0x12: return QStringLiteral("VC_ERR_LENGTH");
    case 0x13: return QStringLiteral("VC_ERR_ADDRESS");
    case 0x14: return QStringLiteral("VC_ERR_VALUE");
    case 0x15: return QStringLiteral("VC_ERR_UNSUPPORTED");
    case 0x16: return QStringLiteral("VC_ERR_BLOCKED");
    case 0x17: return QStringLiteral("VC_ERR_BUSY");
    case 0x18: return QStringLiteral("VC_ERR_STATE");
    case 0x19: return QStringLiteral("VC_ERR_PIN_INVALID");
    case 0x1A: return QStringLiteral("VC_ERR_PIN_IN_USE");
    default: return QString();
    }
}

QString hex4(uint v) { return QStringLiteral("0x%1").arg(v, 4, 16, QLatin1Char('0')).toUpper().replace(QLatin1String("0X"), QLatin1String("0x")); }

// A number as typed: 0x... hexadecimal, else decimal. `max` is what the field
// carries on the wire; a larger one would go out cut, so it is refused.
bool number(const QString &typed, const QString &what, uint max, bool optional, uint &out, QString &error) {
    const QString t = typed.trimmed();
    if (t.isEmpty()) {
        if (optional) {
            out = 0;
            return true;
        }
        error = DeviceInfoController::tr("%1 is missing.").arg(what);
        return false;
    }
    bool ok = false;
    const qulonglong v = t.startsWith(QLatin1String("0x"), Qt::CaseInsensitive) ? t.mid(2).toULongLong(&ok, 16)
                                                                                 : t.toULongLong(&ok, 10);
    if (!ok) {
        error = DeviceInfoController::tr("%1 \"%2\" is not a number: give 0x hexadecimal or decimal.").arg(what, t);
        return false;
    }
    if (v > max) {
        error = DeviceInfoController::tr("%1 %2 is more than %3 bits: it would go out cut, so nothing is sent.")
                    .arg(what, t)
                    .arg(max == 0xFF ? 8 : 16);
        return false;
    }
    out = uint(v);
    return true;
}

// The audio link status fields the sender's page shows, in order
struct AlinkField {
    const char *key;
    const char *label;
};
const AlinkField kAlinkFields[] = {
    { "state", QT_TRANSLATE_NOOP("DeviceInfoController", "State") },
    { "alinkClass", QT_TRANSLATE_NOOP("DeviceInfoController", "Class") },
    { "rateCode", QT_TRANSLATE_NOOP("DeviceInfoController", "Rate code") },
    { "channels", QT_TRANSLATE_NOOP("DeviceInfoController", "Channels") },
    { "hops", QT_TRANSLATE_NOOP("DeviceInfoController", "Hops") },
    { "senderId", QT_TRANSLATE_NOOP("DeviceInfoController", "Sender id") },
    { "frame", QT_TRANSLATE_NOOP("DeviceInfoController", "Frame") },
    { "lockCount", QT_TRANSLATE_NOOP("DeviceInfoController", "Locks") },
    { "lossCount", QT_TRANSLATE_NOOP("DeviceInfoController", "Losses") },
    { "crc8Errors", QT_TRANSLATE_NOOP("DeviceInfoController", "CRC-8 errors") },
    { "crc16Errors", QT_TRANSLATE_NOOP("DeviceInfoController", "CRC-16 errors") },
    { "bitSlips", QT_TRANSLATE_NOOP("DeviceInfoController", "Bit slips") },
    { "superframeErrors", QT_TRANSLATE_NOOP("DeviceInfoController", "Superframe errors") },
    { "vcxoPpm", QT_TRANSLATE_NOOP("DeviceInfoController", "VCXO (ppm)") },
};

} // namespace

DeviceInfoController::DeviceInfoController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &DeviceInfoController::onSnapshot);
    // -- WP QTSENDER -- nothing that needs a DSP runs on the sender
    if (ctx.shell)
        ctx.shell->setActionGate(this, [this](const QString &action) { return !sender() || senderKeeps(action); });
}

// -- WP QTSENDER --
bool DeviceInfoController::senderKeeps(const QString &action) const {
    static const QStringList kept{
        QStringLiteral("file.openFile"),        QStringLiteral("file.newFromTemplate"),  QStringLiteral("file.quit"),
        QStringLiteral("edit.settings"),        QStringLiteral("device.events"),         QStringLiteral("device.identify"),
        QStringLiteral("device.firmwareUpdate"), QStringLiteral("device.reconnect"),     QStringLiteral("device.devices"),
        QStringLiteral("autoeq.clearFavorites"), QStringLiteral("autoeq.updateDatabase"), QStringLiteral("window.minimiseToTray"),
        QStringLiteral("window.system"),
    };
    return action.startsWith(QLatin1String("help.")) || kept.contains(action);
}

DeviceInfoController *DeviceInfoController::create(QQmlEngine *, QJSEngine *) {
    DeviceInfoController *self = Registrar::instance<DeviceInfoController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

QString DeviceInfoController::generation() const {
    return known() ? m_protocol.value(QStringLiteral("generation")).toString() : QString();
}

bool DeviceInfoController::sender() const {
    return known() && m_protocol.value(QStringLiteral("deviceKind")).toString() == QLatin1String("alinkSender");
}

int DeviceInfoController::boardId() const {
    return known() ? m_protocol.value(QStringLiteral("boardId")).toInt(-1) : -1;
}

QString DeviceInfoController::boardName() const {
    return known() ? m_protocol.value(QStringLiteral("boardName")).toString() : QString();
}

// ---- Once per connection ---------------------------------------------------

void DeviceInfoController::onSnapshot() {
    const DeviceSnapshot &s = m_ctx.device->snapshot();
    if (!s.connected) {
        if (known() || m_readFor != 0) clear();
        return;
    }
    if (m_reading || s.generation == m_readFor) return;
    read();
}

void DeviceInfoController::clear() {
    m_protocol = QJsonObject();
    m_rows.clear();
    m_notes.clear();
    m_readFor = 0;
    m_alinkRows.clear();
    m_alinkProblem.clear();
    emit changed();
    emit alinkChanged();
}

void DeviceInfoController::read() {
    m_reading = true;
    m_ctx.core->worker()->call(
        this,
        [](FfiCore *c) {
            const uint64_t generation = dspi_connection_generation(c);
            const QByteArray json = readSized([c](char *b, uint32_t n) { return dspi_device_protocol(c, b, n); });
            return std::make_pair(generation, json);
        },
        [this](std::pair<uint64_t, QByteArray> r) {
            m_reading = false;
            ++m_reads;
            m_readFor = r.first;
            const QJsonObject p = parse(r.second);
            const bool wasSender = sender();
            m_protocol = p.value(QStringLiteral("ok")).toBool() ? p : QJsonObject();
            m_rows = rowsOf(m_protocol);
            m_notes = notesOf(m_protocol);
            m_alinkRows.clear();
            m_alinkProblem.clear();
            emit changed();
            emit alinkChanged();
            if (sender() || wasSender) refreshAlink();
            // A connection that came while this one was read
            onSnapshot();
        });
}

QVariantList DeviceInfoController::rowsOf(const QJsonObject &p) {
    QVariantList rows;
    if (p.isEmpty()) return rows;
    const QString gen = p.value(QStringLiteral("generation")).toString();
    const auto row = [&rows](const char *key, const QString &label, const QString &value) {
        rows.append(QVariantMap{ { QStringLiteral("key"), QString::fromLatin1(key) },
                                 { QStringLiteral("label"), label },
                                 { QStringLiteral("value"), value } });
    };
    const QString name = p.value(QStringLiteral("boardName")).toString();
    const int board = p.value(QStringLiteral("boardId")).toInt();
    row("board", tr("Board"), name.isEmpty() ? tr("board %1").arg(board) : tr("%1 (board %2)").arg(name).arg(board));
    row("generation", tr("Generation"),
        gen == QLatin1String("legacy")  ? tr("legacy: firmware 1.1.6 and older")
        : gen == QLatin1String("paged") ? tr("paged: firmware 1.1.7 and later")
                                        : tr("not known on this connection"));
    row("protocol", tr("Protocol"),
        gen == QLatin1String("paged")    ? tr("paged %1.%2")
                                            .arg(p.value(QStringLiteral("protocolMajor")).toInt())
                                            .arg(p.value(QStringLiteral("protocolMinor")).toInt())
        : gen == QLatin1String("legacy") ? tr("1.1.6 opcodes")
                                         : tr("not known"));
    const QJsonArray pages = p.value(QStringLiteral("pages")).toArray();
    QString pageText;
    if (gen == QLatin1String("paged") && pages.isEmpty()) {
        pageText = tr("none listed on this connection");
    } else if (gen == QLatin1String("paged")) {
        QStringList words;
        for (const QJsonValue &v : pages) {
            const int page = v.toObject().value(QStringLiteral("page")).toInt();
            const int revision = v.toObject().value(QStringLiteral("revision")).toInt();
            const QString pn = pageName(page);
            const QString number = QStringLiteral("0x%1").arg(page, 2, 16, QLatin1Char('0')).toUpper().replace(QLatin1String("0X"), QLatin1String("0x"));
            words << (pn.isEmpty() ? QStringLiteral("%1 r%2").arg(number).arg(revision)
                                   : QStringLiteral("%1 %2 r%3").arg(number, pn).arg(revision));
        }
        pageText = words.join(QLatin1String(", "));
    } else if (gen == QLatin1String("legacy")) {
        pageText = tr("none: page 0 only");
    } else {
        pageText = tr("not known");
    }
    row("pages", tr("Pages"), pageText);
    const QString ext = p.value(QStringLiteral("extEnvelope")).toString();
    row("extensions", tr("Extensions"),
        ext == QLatin1String("legacy")   ? tr("REQ_EXTENSION 0x00 on page 0")
        : ext == QLatin1String("tunnel") ? tr("tunnel 0xE010 on page 0xE0")
                                         : tr("none (no extension channel)"));
    return rows;
}

QStringList DeviceInfoController::notesOf(const QJsonObject &p) {
    QStringList notes;
    if (p.isEmpty()) return notes;
    if (p.value(QStringLiteral("generation")).toString() == QLatin1String("paged") &&
        p.value(QStringLiteral("pages")).toArray().isEmpty())
        notes << tr("This connection says the device is paged but carries no page list: a hub's in-process or "
                    "principal connection does not pass the page discovery on. The page list is therefore empty "
                    "here, and the extension channel is as the 1.1.6-shaped probe found it.");
    if (p.value(QStringLiteral("deviceKind")).toString() == QLatin1String("alinkSender"))
        notes << tr("This device is the audio link sender, not a DSP: it has its device information, its extension "
                    "channel with the audio link settings, and firmware updates. It has no DSP settings, no meters "
                    "and no signal generator.");
    if (p.value(QStringLiteral("boardId")).toInt() != 0)
        notes << tr("This board has no BOOTSEL bootloader that this app can write: it is offered no firmware image.");
    return notes;
}

// ---- The raw dialog --------------------------------------------------------

DeviceInfoController::RawRequest DeviceInfoController::parseRaw(bool set, const QString &id, const QString &value,
                                                                const QString &lengthOrHex) {
    RawRequest r;
    uint n = 0;
    if (!number(id, tr("The command id"), 0xFFFF, false, n, r.error)) return r;
    r.id = uint16_t(n);
    if (!number(value, tr("wValue"), 0xFFFF, true, n, r.error)) return r;
    r.value = uint16_t(n);
    if (!set) {
        if (!number(lengthOrHex, tr("The length"), 0xFFFF, false, n, r.error)) return r;
        r.length = uint16_t(n);
        return r;
    }
    // The payload: bytes as hex, "01 02 ff", "0x01,0x02" or "0102ff"
    QStringList tokens = lengthOrHex.split(QRegularExpression(QStringLiteral("[\\s,]+")), Qt::SkipEmptyParts);
    static const QRegularExpression hexDigits(QStringLiteral("^[0-9A-Fa-f]+$"));
    for (const QString &raw : std::as_const(tokens)) {
        const bool prefixed = raw.startsWith(QLatin1String("0x"), Qt::CaseInsensitive);
        const QString t = prefixed ? raw.mid(2) : raw;
        if (!hexDigits.match(t).hasMatch()) {
            r.error = tr("\"%1\" is not hex bytes: give the payload as hex, e.g. 01 ff.").arg(raw);
            return r;
        }
        // 0x1FF, or an odd run of digits, names more than whole bytes
        if ((prefixed && t.size() > 2) || (t.size() > 1 && t.size() % 2 != 0)) {
            r.error = tr("The byte %1 is more than 8 bits: it would go out cut, so nothing is sent.").arg(raw);
            r.data.clear();
            return r;
        }
        for (int at = 0; at < t.size(); at += 2) r.data.append(char(t.mid(at, 2).toUInt(nullptr, 16)));
    }
    if (r.data.size() > 0xFFFF) {
        r.error = tr("The payload is %1 bytes; a control transfer carries at most 65535, so nothing is sent.")
                      .arg(r.data.size());
        r.data.clear();
    }
    return r;
}

QString DeviceInfoController::refusalWords(uint16_t id, const QJsonObject &reply) {
    const QString name = hex4(id);
    const QString code = reply.value(QStringLiteral("code")).toString();
    QString error = reply.value(QStringLiteral("error")).toString();
    if (code == QLatin1String("invalid")) {
        static const QString prefix = QStringLiteral("invalid argument: ");
        if (error.startsWith(prefix)) error = error.mid(prefix.size());
        return tr("%1 was not sent: %2").arg(name, error);
    }
    if (code == QLatin1String("unsupported"))
        return tr("%1 was not sent: this device speaks the 1.1.6 protocol, which has no pages (or this connection "
                  "does not say it is paged). A request by command id needs firmware 1.1.7 or later.")
            .arg(name);
    if (code == QLatin1String("refused")) {
        static const QRegularExpression status(QStringLiteral("status (\\d+)"));
        const QRegularExpressionMatch m = status.match(error);
        const int s = m.hasMatch() ? m.captured(1).toInt() : 0;
        if (s == 0) return tr("The device refused %1; it gave no result code.").arg(name);
        const QString rn = resultName(s);
        return tr("The device refused %1: %2.")
            .arg(name, rn.isEmpty() ? tr("result code %1").arg(s) : QStringLiteral("%1 (%2)").arg(rn).arg(s));
    }
    if (error.isEmpty()) error = tr("no reply");
    return tr("%1 was not sent: %2").arg(name, error);
}

QString DeviceInfoController::hexDump(const QByteArray &bytes) {
    QStringList lines;
    for (int at = 0; at < bytes.size(); at += 16) {
        const QByteArray chunk = bytes.mid(at, 16);
        QStringList hex;
        QString ascii;
        for (const char ch : chunk) {
            const uchar b = uchar(ch);
            hex << QStringLiteral("%1").arg(b, 2, 16, QLatin1Char('0')).toUpper();
            ascii += (b > 0x20 && b < 0x7F) ? QChar(b) : QLatin1Char('.');
        }
        lines << QStringLiteral("%1  %2  %3")
                     .arg(QStringLiteral("%1").arg(at, 4, 16, QLatin1Char('0')).toUpper())
                     .arg(hex.join(QLatin1Char(' ')), -47)
                     .arg(ascii);
    }
    return lines.join(QLatin1Char('\n'));
}

void DeviceInfoController::rawDone(bool failed, const QString &text) {
    m_rawBusy = false;
    m_rawFailed = failed;
    m_rawResult = text;
    emit rawChanged();
}

void DeviceInfoController::rawGet(const QString &id, const QString &value, const QString &length) {
    if (m_rawBusy) return;
    const RawRequest r = parseRaw(false, id, value, length);
    if (!r.error.isEmpty()) {
        rawDone(true, r.error);
        return;
    }
    m_rawBusy = true;
    emit rawChanged();
    const uint16_t cmd = r.id, wValue = r.value, want = r.length;
    m_ctx.core->worker()->call(
        this,
        [cmd, wValue, want](FfiCore *c) {
            return readSized([&](char *b, uint32_t n) { return dspi_raw_get_id(c, cmd, wValue, want, b, n); });
        },
        [this, cmd](QByteArray json) {
            const QJsonObject reply = parse(json);
            if (!reply.value(QStringLiteral("ok")).toBool()) {
                rawDone(true, refusalWords(cmd, reply));
                return;
            }
            QByteArray bytes;
            for (const QJsonValue &v : reply.value(QStringLiteral("value")).toArray()) bytes.append(char(v.toInt()));
            rawDone(false, (bytes.isEmpty() ? QString() : hexDump(bytes) + QLatin1Char('\n')) +
                               (bytes.size() == 1 ? tr("1 byte from %1").arg(hex4(cmd))
                                                  : tr("%1 bytes from %2").arg(bytes.size()).arg(hex4(cmd))));
        });
}

void DeviceInfoController::rawSet(const QString &id, const QString &value, const QString &hex) {
    if (m_rawBusy) return;
    const RawRequest r = parseRaw(true, id, value, hex);
    if (!r.error.isEmpty()) {
        rawDone(true, r.error);
        return;
    }
    m_rawBusy = true;
    emit rawChanged();
    const uint16_t cmd = r.id, wValue = r.value;
    const QByteArray data = r.data;
    auto reply = std::make_shared<QByteArray>();
    // The one write path; a name of its own, so it never stands in for another
    m_ctx.device->write(
        QStringLiteral("raw set %1 #%2").arg(hex4(cmd)).arg(++m_rawCount),
        [reply, cmd, wValue, data](FfiCore *c) {
            *reply = readSized([&](char *b, uint32_t n) {
                return dspi_raw_set_id(c, cmd, wValue, data.isEmpty() ? nullptr : reinterpret_cast<const uint8_t *>(data.constData()),
                                       uint32_t(data.size()), b, n);
            });
            return parse(*reply).value(QStringLiteral("ok")).toBool();
        },
        [this, reply, cmd, n = int(data.size())](bool ok) {
            if (ok) {
                rawDone(false, n == 1 ? tr("Sent 1 byte to %1.").arg(hex4(cmd)) : tr("Sent %1 bytes to %2.").arg(n).arg(hex4(cmd)));
            } else if (reply->isEmpty()) {
                rawDone(true, tr("%1 was not sent: no device is open, or it is read-only.").arg(hex4(cmd)));
            } else {
                rawDone(true, refusalWords(cmd, parse(*reply)));
            }
        });
}

// ---- The alink sender's page -----------------------------------------------

void DeviceInfoController::refreshAlink() {
    if (m_alinkBusy) return;
    m_alinkBusy = true;
    emit alinkChanged();
    m_ctx.core->worker()->call(
        this, [](FfiCore *c) { return readSized([c](char *b, uint32_t n) { return dspi_fetch_alink_status(c, b, n); }); },
        [this](QByteArray json) {
            m_alinkBusy = false;
            m_alinkRows.clear();
            m_alinkProblem.clear();
            const QJsonObject reply = parse(json);
            if (reply.value(QStringLiteral("ok")).toBool()) {
                for (const AlinkField &f : kAlinkFields) {
                    const QJsonValue v = reply.value(QLatin1String(f.key));
                    if (v.isUndefined() || v.isNull()) continue;
                    m_alinkRows.append(QVariantMap{
                        { QStringLiteral("label"), QCoreApplication::translate("DeviceInfoController", f.label) },
                        { QStringLiteral("value"), v.isString() ? v.toString() : v.toVariant().toString() } });
                }
            } else {
                const QString code = reply.value(QStringLiteral("code")).toString();
                if (code == QLatin1String("denied"))
                    m_alinkProblem = tr("The audio link status is for admin sessions on this hub: its extension channel "
                                        "carries settings that change the hardware.");
                else if (code == QLatin1String("unsupported"))
                    m_alinkProblem = tr("This device has no audio link block.");
                else if (code == QLatin1String("disconnected") || json.isEmpty())
                    m_alinkProblem = tr("No device is open.");
                else
                    m_alinkProblem = tr("The audio link status could not be read: %1")
                                         .arg(reply.value(QStringLiteral("error")).toString());
            }
            emit alinkChanged();
        });
}

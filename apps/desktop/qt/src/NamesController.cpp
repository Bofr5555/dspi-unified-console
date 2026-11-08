#include "NamesController.h"
#include "DSPiBridge.h"
#include "Registrar.h"

#include <QRunnable>

static const Registrar::Controller<NamesController> namesRegistered("deviceNames");

namespace {
constexpr int kNameLen = 32;
constexpr int kDeviceWrite = -1;

QString trimmedName(const QString &text) {
    return text.trimmed();
}
}

NamesController::NamesController(DSPiBridge *bridge, QObject *parent)
    : QObject(parent), m_bridge(bridge), m_core(bridge->core())
{
    for (int i = 0; i < kSourceCount; i++) m_sources.append(QString());
    m_worker.setMaxThreadCount(1);
    // A disconnect or a connect comes as stateChanged, a list change as devicesChanged
    connect(m_bridge, &DSPiBridge::devicesChanged, this, &NamesController::sync);
    connect(m_bridge, &DSPiBridge::stateChanged, this, &NamesController::sync);
    connect(m_bridge, &DSPiBridge::notificationsArrived, this, &NamesController::refresh);
    // -- WP QD6 -- Before the next device opens, not after: stateChanged comes too late
    connect(m_bridge, &DSPiBridge::deviceChanging, this, [this]() { m_generation++; });
    sync();
}

NamesController::~NamesController() {
    m_generation++;
    m_worker.waitForDone();
}

// A new device (or none): its names, without the old one's drafts
void NamesController::sync() {
    const QString serial = m_bridge->connected() ? m_bridge->selectedSerial() : QString();
    if (serial != m_serial) {
        m_serial = serial;
        m_generation++;
        clearDrafts();
    }
    refresh();
}

// No traffic, so it can run on every batch of notifications
void NamesController::refresh() {
    char buf[kNameLen];
    QStringList sources;
    bool sourcesSupported = false;
    for (int i = 0; i < kSourceCount; i++) {
        const bool ok = dspi_cached_input_source_name(m_core, uint8_t(i), buf, kNameLen);
        sources.append(ok ? QString::fromUtf8(buf) : QString());
        sourcesSupported = sourcesSupported || ok;
    }
    const bool deviceSupported = dspi_cached_device_name(m_core, buf, kNameLen);
    const QString device = deviceSupported ? QString::fromUtf8(buf) : QString();
    if (sources == m_sources && device == m_device && sourcesSupported == m_sourcesSupported
        && deviceSupported == m_deviceSupported)
        return;
    m_sources = sources;
    m_device = device;
    m_sourcesSupported = sourcesSupported;
    m_deviceSupported = deviceSupported;
    emit storedChanged();
    emit textsChanged();
}

QStringList NamesController::sourceTexts() const {
    QStringList texts = m_sources;
    for (int i = 0; i < kSourceCount; i++)
        if (m_sourceDrafts[i].held) texts[i] = m_sourceDrafts[i].text;
    return texts;
}

QString NamesController::deviceText() const {
    return m_deviceDraft.held ? m_deviceDraft.text : m_device;
}

bool NamesController::sourcePending(int source) const {
    if (source < 0 || source >= kSourceCount || !m_sourcesSupported) return false;
    const Draft &d = m_sourceDrafts[source];
    return d.held && trimmedName(d.text) != m_sources[source];
}

bool NamesController::devicePending() const {
    return m_deviceSupported && m_deviceDraft.held && trimmedName(m_deviceDraft.text) != m_device;
}

bool NamesController::dirty() const {
    for (int i = 0; i < kSourceCount; i++)
        if (sourcePending(i)) return true;
    return devicePending();
}

QString NamesController::clean(const QString &text) const {
    QString out;
    for (const QChar c : text) {
        if (out.size() == kNameLen - 1) break;
        if (c.unicode() >= 32 && c.unicode() < 127) out.append(c);
    }
    return out;
}

void NamesController::editSource(int source, const QString &text) {
    if (source < 0 || source >= kSourceCount || !m_bridge->writable()) return;   // -- WP QD6 --
    const QString name = clean(text);
    Draft &d = m_sourceDrafts[source];
    d.held = name != m_sources[source];
    d.text = d.held ? name : QString();
    emit textsChanged();
}

void NamesController::editDevice(const QString &text) {
    if (!m_bridge->writable()) return;   // -- WP QD6 --
    const QString name = clean(text);
    m_deviceDraft.held = name != m_device;
    m_deviceDraft.text = m_deviceDraft.held ? name : QString();
    emit textsChanged();
}

void NamesController::clearDrafts() {
    for (Draft &d : m_sourceDrafts) d = Draft();
    m_deviceDraft = Draft();
    emit textsChanged();
}

void NamesController::revert() {
    clearDrafts();
}

void NamesController::save() {
    if (m_saving || !dirty() || !m_bridge->writable()) return;   // -- WP QD6 --
    std::vector<Write> writes;
    for (int i = 0; i < kSourceCount; i++)
        if (sourcePending(i)) writes.push_back({ i, trimmedName(m_sourceDrafts[i].text).toUtf8() });
    if (devicePending()) writes.push_back({ kDeviceWrite, trimmedName(m_deviceDraft.text).toUtf8() });
    m_saving = true;
    emit savingChanged();
    saveDrafts(writes);
}

// The one place names reach the device
void NamesController::saveDrafts(const std::vector<Write> &writes) {
    const unsigned generation = m_generation;
    QRunnable *job = QRunnable::create([this, writes, generation]() {
        bool ok = true;
        for (const Write &w : writes) {
            if (m_writeHook && !m_writeHook()) { ok = false; break; }
            // A changed device gets none of the old one's names
            if (m_generation != generation) break;
            ok = w.source == kDeviceWrite
                ? dspi_set_device_name(m_core, w.name.constData())
                : dspi_set_input_source_name(m_core, uint8_t(w.source), w.name.constData());
            if (!ok) break;
        }
        QMetaObject::invokeMethod(this, [this, writes, ok, generation]() {
            m_saving = false;
            emit savingChanged();
            if (m_generation == generation) saveDone(writes, ok);
        }, Qt::QueuedConnection);
    });
    m_worker.start(job);
}

void NamesController::saveDone(const std::vector<Write> &writes, bool ok) {
    refresh();      // what went out is in the cache now
    // A draft edited again during the save stays. So does one the refusal left
    // unsent, as it differs from what is stored.
    for (const Write &w : writes) {
        const QString sent = QString::fromUtf8(w.name);
        if (w.source == kDeviceWrite) {
            if (m_deviceDraft.held && trimmedName(m_deviceDraft.text) == sent && sent == m_device)
                m_deviceDraft = Draft();
        } else {
            Draft &d = m_sourceDrafts[w.source];
            if (d.held && trimmedName(d.text) == sent && sent == m_sources[w.source])
                d = Draft();
        }
    }
    emit textsChanged();
    if (!ok) emit saveFailed(tr("The device did not take the name."));
}

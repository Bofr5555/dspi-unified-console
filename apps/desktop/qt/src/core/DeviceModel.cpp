#include "DeviceModel.h"

#include "ChannelListModel.h"
#include "Core.h"
#include "DeviceRegistry.h"
#include "PresetModel.h"
#include "SessionModel.h"

#include <QGuiApplication>
#include <QJSEngine>
#include <QQuickItem>
#include <QQuickWindow>

namespace {
DeviceModel *s_instance = nullptr;
} // namespace

// -- WP MDQT1 -- The facade over the active session's model (SessionModel)
DeviceModel::DeviceModel(Core *core, DeviceRegistry *registry, QObject *parent)
    : QObject(parent), m_core(core), m_registry(registry), m_channels(new ChannelListModel(this)),
      m_presets(new PresetModel(this)) {
    s_instance = this;
    connect(registry, &DeviceRegistry::activeLeaving, this, &DeviceModel::letGo);   // -- WP MDQT2a --
    connect(registry, &DeviceRegistry::activeMoved, this, &DeviceModel::rebind);
    rebind();
}

DeviceModel::~DeviceModel() {
    if (s_instance == this) s_instance = nullptr;
}

DeviceModel *DeviceModel::instance() { return s_instance; }

DeviceModel *DeviceModel::create(QQmlEngine *, QJSEngine *) {
    QJSEngine::setObjectOwnership(s_instance, QJSEngine::CppOwnership);
    return s_instance;
}

// -- WP MDQT2a -- The device is about to be left: a field being typed in loses
// the focus, so that it commits (or drops) its edit as on a click elsewhere, on
// the device it was made on. (A held slider lets go by itself, CustomSlider.)
void DeviceModel::letGo() {
    for (QWindow *w : QGuiApplication::allWindows()) {
        auto *window = qobject_cast<QQuickWindow *>(w);
        if (!window) continue;
        QQuickItem *typed = window->activeFocusItem();
        if (typed && (typed->inherits("QQuickTextInput") || typed->inherits("QQuickTextEdit"))) typed->setFocus(false);
    }
}

void DeviceModel::rebind() {
    SessionModel *before = m_active;
    m_active = m_registry->active()->model();
    if (before == m_active) return;
    const bool wasBusy = before && before->busy();
    if (before) {
        disconnect(before, nullptr, this, nullptr);
        before->endDrag();   // a handle held over the switch moves nothing more
    }
    connect(m_active, &SessionModel::applied, this, &DeviceModel::onApplied);
    connect(m_active, &SessionModel::busyChanged, this, &DeviceModel::busyChanged);
    connect(m_active, &SessionModel::writeFailed, this, &DeviceModel::writeFailed);
    connect(m_active, &SessionModel::writeFinished, this, &DeviceModel::writeFinished);
    connect(m_active, &SessionModel::writeRefused, this, &DeviceModel::writeRefused);
    if (!before) return;
    onApplied(0);
    if (wasBusy != m_active->busy()) emit busyChanged();
}

void DeviceModel::onApplied(uint flags) {
    ++m_revision;
    m_channels->update(m_active->snapshot(), platformName());
    m_presets->update(m_active->snapshot());
    emit snapshotApplied();
    emit stateChanged(flags);
}

bool DeviceModel::connected() const { return m_active->connected(); }
bool DeviceModel::demo() const { return m_active->demo(); }
bool DeviceModel::readOnly() const { return m_active->readOnly(); }
QString DeviceModel::serial() const { return m_active->serial(); }
QString DeviceModel::deviceName() const { return m_active->deviceName(); }
QString DeviceModel::deviceTitle() const { return m_active->deviceTitle(); }
QString DeviceModel::platformName() const { return m_active->platformName(); }
QString DeviceModel::firmwareVersion() const { return m_active->firmwareVersion(); }
int DeviceModel::wireFormat() const { return m_active->wireFormat(); }
int DeviceModel::compat() const { return m_active->compat(); }
int DeviceModel::numInputs() const { return m_active->numInputs(); }
int DeviceModel::numOutputs() const { return m_active->numOutputs(); }
bool DeviceModel::busy() const { return m_active->busy(); }
bool DeviceModel::remote() const { return m_active->remote(); }
bool DeviceModel::document() const { return m_active->document(); }
bool DeviceModel::dragging() const { return m_active->dragging(); }
const DspState *DeviceModel::state() const { return m_active->state(); }
const DeviceSnapshot &DeviceModel::snapshot() const { return m_active->snapshot(); }
int DeviceModel::settersRun() const { return m_active->settersRun(); }

void DeviceModel::start(bool demo) {
    m_registry->mainSession()->model()->start(demo);
    m_core->startHotplug();
}

void DeviceModel::openDemo() { m_active->openDemo(); }
void DeviceModel::resetDemo() { m_active->resetDemo(); }   // -- WP UB12 --
void DeviceModel::selectDevice(const QString &serial) { m_active->selectDevice(serial); }
void DeviceModel::reconnect() { m_active->reconnect(); }
void DeviceModel::refresh() { m_active->refresh(); }
void DeviceModel::reread() { m_active->reread(); }   // -- WP SHOWFIX --
void DeviceModel::adoptDocument(const QString &serial) { m_active->adoptDocument(serial); }

void DeviceModel::write(const QString &what, std::function<bool(FfiCore *)> setter, std::function<void(bool)> done) {
    m_active->write(what, std::move(setter), std::move(done));
}

void DeviceModel::writeEvenReadOnly(const QString &what, std::function<bool(FfiCore *)> setter,
                                    std::function<void(bool)> done) {
    m_active->writeEvenReadOnly(what, std::move(setter), std::move(done));
}

void DeviceModel::beginDrag() { m_active->beginDrag(); }
void DeviceModel::endDrag() { m_active->endDrag(); }

bool DeviceModel::setChannelName(int channel, const QString &name) { return m_active->setChannelName(channel, name); }

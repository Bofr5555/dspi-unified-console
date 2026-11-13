#include "ModeController.h"

#include "DeviceModel.h"

#include <QJSEngine>
#include <QSettings>

static const Registrar::Controller<ModeController> modeController;
static const Registrar::Window modeWindow("window.mode", "ModeWindow");

ModeController::ModeController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &ModeController::changed);
}

ModeController *ModeController::create(QQmlEngine *, QJSEngine *) {
    ModeController *self = Registrar::instance<ModeController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

bool ModeController::known(const QString &mode) {
    return mode == QLatin1String("simple") || mode == QLatin1String("advanced");
}

QString ModeController::deviceKey() const {
    const QString serial = m_ctx.device->connected() ? m_ctx.device->serial() : QString();
    return serial.isEmpty() ? QString() : QStringLiteral("mode/device/") + serial;
}

QString ModeController::mode() const {
    QSettings settings;
    const QString key = deviceKey();
    const QString own = key.isEmpty() ? QString() : settings.value(key).toString();
    if (known(own)) return own;
    const QString fallback = settings.value(QStringLiteral("mode/default")).toString();
    return known(fallback) ? fallback : QStringLiteral("advanced");
}

QString ModeController::modeTitle() const {
    return mode() == QLatin1String("simple") ? tr("Simple") : tr("Advanced");
}

QVariantList ModeController::modes() const {
    return { QVariantMap{ { QStringLiteral("id"), QStringLiteral("simple") }, { QStringLiteral("title"), tr("Simple") } },
             QVariantMap{ { QStringLiteral("id"), QStringLiteral("advanced") }, { QStringLiteral("title"), tr("Advanced") } } };
}

bool ModeController::rememberedForDevice() const {
    const QString key = deviceKey();
    return !key.isEmpty() && known(QSettings().value(key).toString());
}

void ModeController::choose(const QString &mode, bool forEveryDevice) {
    if (!known(mode)) return;
    QSettings settings;
    const QString key = deviceKey();
    if (forEveryDevice || key.isEmpty()) {
        settings.setValue(QStringLiteral("mode/default"), mode);
        if (!key.isEmpty()) settings.remove(key);
    } else {
        settings.setValue(key, mode);
    }
    emit changed();
}

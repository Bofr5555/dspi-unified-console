#include "ShortcutController.h"

#include "DeviceLinkController.h"   // -- WP UB34 --
#include "CoreApi.h"
#include "DeviceModel.h"
#include "Shell.h"
#include "SidebarController.h"

#include <QJSEngine>

static const Registrar::Controller<ShortcutController> shortcutController;

ShortcutController::ShortcutController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    for (int n = 1; n <= 9; ++n)
        ctx.shell->addHandler(QStringLiteral("device.preset%1").arg(n), this, [this, n] { recallPreset(n); });
    ctx.shell->addHandler(QStringLiteral("device.mute"), this, [this] { toggleMute(); });
    ctx.shell->addHandler(QStringLiteral("device.bypass"), this, [this] { toggleBypass(); });
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &ShortcutController::changed);
}

ShortcutController *ShortcutController::create(QQmlEngine *, QJSEngine *) {
    ShortcutController *self = Registrar::instance<ShortcutController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

bool ShortcutController::muted() const {
    const DspState *s = m_ctx.device->state();
    return s && s->user_mute;
}

bool ShortcutController::bypassed() const {
    const DspState *s = m_ctx.device->state();
    return s && s->bypass;
}

void ShortcutController::recallPreset(int number) {
    if (number < 1 || number > 9 || !m_ctx.device->connected()) return;
    if (SidebarController *sidebar = Registrar::instance<SidebarController>()) sidebar->requestPresetSwitch(number - 1);
}

void ShortcutController::toggleMute() {
    if (!m_ctx.device->connected()) return;
    const bool on = !muted();
    // -- WP UB34 -- (UXP B4) linked in the dock: Mute mutes every device
    if (auto *link = Registrar::instance<DeviceLinkController>(); link && link->linked()) return link->setMute(on);
    m_ctx.device->write(QStringLiteral("user mute"), [on](FfiCore *c) { return dspi_set_user_mute(c, on); });
}

void ShortcutController::toggleBypass() {
    if (!m_ctx.device->connected()) return;
    if (SidebarController *sidebar = Registrar::instance<SidebarController>())
        sidebar->setEffect(QStringLiteral("bypass"), !sidebar->bypassOn());
}

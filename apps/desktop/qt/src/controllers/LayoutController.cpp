// -- WP LAY0 -- The layout switch (LayoutController.h)
#include "LayoutController.h"

#include "PrefsMap.h"
#include "SettingsController.h"

#include <QSettings>

namespace {

const Registrar::Controller<LayoutController> layoutController;
// Settings > Appearance: hidden until the Modern shell is there to switch to
// (WP LAY3 moves it into the Application group); SettingsController.open("appearance")
// shows it meanwhile
const SettingsRegistry::Page appearancePage{
    "appearance", SettingsRegistry::Hidden, 15, QT_TRANSLATE_NOOP("SettingsPages", "Appearance"), "layout-grid",
    "AppearancePage",
    { QT_TRANSLATE_NOOP("SettingsPages", "Layout"), QT_TRANSLATE_NOOP("SettingsPages", "Classic"),
      QT_TRANSLATE_NOOP("SettingsPages", "Modern") }
};

const char *const kSetupDoneKey = "onboarding/setupDone";   // OnboardingController's

bool known(const QString &layout) { return layout == QLatin1String("classic") || layout == QLatin1String("modern"); }

} // namespace

LayoutController::LayoutController(const AppContext &, QObject *parent)
    : QObject(parent), m_arrangement(new PrefsMap(QStringLiteral("ui/arrangement"), this)) {
    m_arrangement->declare(QStringLiteral("navigation"), QStringLiteral("rail"));
    m_arrangement->declare(QStringLiteral("deviceTabs"), QStringLiteral("topBar"));

    QSettings settings;
    m_layout = settings.value(QLatin1String(kLayoutKey)).toString();
    if (!known(m_layout)) {
        m_layout = settings.value(QLatin1String(kSetupDoneKey), false).toBool() ? QStringLiteral("classic")
                                                                                 : QStringLiteral("modern");
        settings.setValue(QLatin1String(kLayoutKey), m_layout);
    }
}

void LayoutController::setLayout(const QString &layout) {
    if (!known(layout) || layout == m_layout) return;
    m_layout = layout;
    QSettings().setValue(QLatin1String(kLayoutKey), m_layout);
    emit layoutChanged();
}

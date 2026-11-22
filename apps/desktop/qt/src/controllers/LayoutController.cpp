// -- WP LAY0 -- The layout switch (LayoutController.h)
#include "LayoutController.h"

#include "MeasureController.h"   // -- WP RBFIX --
#include "NoticeCenter.h"
#include "PrefsMap.h"
#include "SettingsController.h"
#include "Shell.h"

#include <QSettings>

namespace {

const Registrar::Controller<LayoutController> layoutController;
// Settings > Appearance
const SettingsRegistry::Page appearancePage{
    "appearance", SettingsRegistry::Application, 35, QT_TRANSLATE_NOOP("SettingsPages", "Appearance"), "layout-grid",
    "AppearancePage",
    { QT_TRANSLATE_NOOP("SettingsPages", "Layout"), QT_TRANSLATE_NOOP("SettingsPages", "Classic"),
      QT_TRANSLATE_NOOP("SettingsPages", "Modern"), QT_TRANSLATE_NOOP("SettingsPages", "Theme"),
      QT_TRANSLATE_NOOP("SettingsPages", "Dark"), QT_TRANSLATE_NOOP("SettingsPages", "Light"),
      // -- WP LAY4 --
      QT_TRANSLATE_NOOP("SettingsPages", "Darker grey"), QT_TRANSLATE_NOOP("SettingsPages", "Inspector"),
      QT_TRANSLATE_NOOP("SettingsPages", "Band bar") }
};

const char *const kSetupDoneKey = "onboarding/setupDone";   // OnboardingController's

bool known(const QString &layout) { return layout == QLatin1String("classic") || layout == QLatin1String("modern"); }

// -- WP LAY3 --
Qt::ColorScheme schemeOf(const QString &theme) {
    return theme == QLatin1String("light") ? Qt::ColorScheme::Light
         : theme == QLatin1String("dark") ? Qt::ColorScheme::Dark : Qt::ColorScheme::Unknown;
}

} // namespace

LayoutController::LayoutController(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_ctx(ctx), m_arrangement(new PrefsMap(QStringLiteral("ui/arrangement"), this)) {
    m_arrangement->declare(QStringLiteral("navigation"), QStringLiteral("rail"));
    m_arrangement->declare(QStringLiteral("deviceTabs"), QStringLiteral("topBar"));
    // -- WP LAY4 --
    m_arrangement->declare(QStringLiteral("tone"), QStringLiteral("grey"));
    m_arrangement->declare(QStringLiteral("inspector"), QStringLiteral("shown"));
    m_arrangement->declare(QStringLiteral("bandBar"), QStringLiteral("hidden"));

    QSettings settings;
    m_layout = settings.value(QLatin1String(kLayoutKey)).toString();
    if (!known(m_layout)) {
        m_layout = settings.value(QLatin1String(kSetupDoneKey), false).toBool() ? QStringLiteral("classic")
                                                                                 : QStringLiteral("modern");
        settings.setValue(QLatin1String(kLayoutKey), m_layout);
    }
    // -- WP LAY3 --
    const QString theme = settings.value(QLatin1String(kThemeKey)).toString();
    if (theme == QLatin1String("light") || theme == QLatin1String("dark")) {
        m_theme = theme;
        if (qEnvironmentVariableIsEmpty("DSPI_COLOR_SCHEME")) m_ctx.shell->setColorSchemeOverride(schemeOf(m_theme));
    }
    // -- WP RBFIX --
    connect(this, &LayoutController::layoutChanged, this, &LayoutController::switchBlockerChanged);
    connect(ctx.shell, &Shell::shownWindowsChanged, this, &LayoutController::switchBlockerChanged);
    QMetaObject::invokeMethod(
        this,
        [this] {
            auto *measure = Registrar::instance<MeasureController>();
            if (!measure) return;
            connect(measure, &MeasureController::runsChanged, this, &LayoutController::switchBlockerChanged);
            emit switchBlockerChanged();
        },
        Qt::QueuedConnection);
}

// -- WP RBFIX -- Only where the switch would stop them: the Measure panel on its
// page in Modern. A window of its own stays open over the switch.
QString LayoutController::switchBlocker() const {
    if (!modern() || m_ctx.shell->shownWindows().contains(QLatin1String("tools.measure"))) return QString();
    const auto *measure = Registrar::instance<MeasureController>();
    const QString on = measure ? measure->runsOn() : QString();
    //: %1 is a device's name (or several)
    return on.isEmpty() ? QString() : tr("A measurement is running on %1. Switching the layout would stop it: stop it first.").arg(on);
}

void LayoutController::setLayout(const QString &layout) {
    if (!known(layout) || layout == m_layout) return;
    // -- WP RBFIX --
    if (const QString why = switchBlocker(); !why.isEmpty()) {
        if (auto *notices = Registrar::instance<NoticeCenter>()) notices->post(why, QStringLiteral("warning"));
        return;
    }
    emit layoutChanging();   // -- WP LAY3 --
    m_layout = layout;
    QSettings().setValue(QLatin1String(kLayoutKey), m_layout);
    emit layoutChanged();
}

// -- WP LAY3 --
void LayoutController::setTheme(const QString &theme) {
    if (theme != QLatin1String("system") && theme != QLatin1String("light") && theme != QLatin1String("dark")) return;
    if (theme == m_theme) return;
    m_theme = theme;
    QSettings().setValue(QLatin1String(kThemeKey), m_theme);
    m_ctx.shell->setColorSchemeOverride(schemeOf(m_theme));
    emit themeChanged();
}

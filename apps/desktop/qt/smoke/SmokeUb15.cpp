// -- WP UB15 -- (UXP B15) Connecting Claude Desktop. "ub15-pictures" grabs
// Settings › AI Agents with no agent paired and the Claude Desktop choice, in
// Modern at 1100×900, in the dark and the light scheme into <out>/ub15/. It
// drives the app through properties only, so the same walk runs on a tree
// without this package (the "before" pictures).
#include "SmokeDriver.h"

#include "AiController.h"
#include "Registrar.h"
#include "Shell.h"

#include <QDir>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTcpServer>
#include <utility>

namespace {

int freePort() {
    QTcpServer probe;
    return probe.listen(QHostAddress::LocalHost, 0) ? int(probe.serverPort()) : 0;
}

// The window in both schemes, as <out>/ub15/<name>-<scheme>.png
void grabBoth(SmokeDriver &d, QQuickWindow *w, const QString &name) {
    Shell *shell = d.app().shell;
    const Qt::ColorScheme was = shell->colorSchemeOverride();
    for (const auto &[scheme, word] : { std::pair{ Qt::ColorScheme::Dark, "dark" }, std::pair{ Qt::ColorScheme::Light, "light" } }) {
        shell->setColorSchemeOverride(scheme);
        d.spin(300);
        d.grab(w, QStringLiteral("ub15/%1-%2.png").arg(name, QLatin1String(word)));
    }
    shell->setColorSchemeOverride(was);
    d.spin(150);
}

void picturesSection(SmokeDriver &d) {
    auto *ai = Registrar::instance<AiController>();
    Shell *shell = d.app().shell;
    if (!ai || !shell) {
        d.check("ub15-pictures: the controllers exist", false);
        return;
    }
    QDir(QDir(d.outDir()).filePath(QStringLiteral("ub15"))).removeRecursively();
    QDir().mkpath(QDir(d.outDir()).filePath(QStringLiteral("ub15")));
    d.switchLayout("modern");
    ai->setStoreDir(QDir(d.outDir()).filePath(QStringLiteral("store-ub15")));
    ai->setPort(freePort());
    ai->setEnabled(true);
    d.waitFor(10000, [&] { return ai->running() && !ai->busy(); });
    ai->openSettings();
    auto *settings = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("edit.settings")));
    d.check("ub15-pictures: Settings opens", settings != nullptr);
    if (settings) {
        settings->resize(1100, 900);
        d.waitFor(3000, [&] { return d.item(QStringLiteral("AiAgentsPage"), settings->contentItem()) != nullptr; });
        d.spin(500);
        grabBoth(d, settings, QStringLiteral("1-claude-desktop"));
        settings->close();
    }
    ai->setEnabled(false);
    d.waitFor(10000, [&] { return !ai->running() && !ai->busy(); });
    ai->setPort(AiController::kDefaultPort);
    d.waitIdle();
}

const SmokeDriver::Section pictures("ub15-pictures", 999, &picturesSection);

} // namespace

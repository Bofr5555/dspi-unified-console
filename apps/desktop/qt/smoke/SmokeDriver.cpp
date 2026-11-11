#include "SmokeDriver.h"

#include "Core.h"
#include "DeviceModel.h"

#include <QCoreApplication>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QImage>
#include <QQmlApplicationEngine>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTimer>
#include <algorithm>
#include <atomic>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <mutex>
#include <string>
#include <thread>
#include <utility>
#include <vector>


// QML runtime errors fail the run (TypeError, ReferenceError, a broken binding)
static int g_qmlErrors = 0;
static QtMessageHandler g_previous = nullptr;

static void messageHandler(QtMsgType type, const QMessageLogContext &ctx, const QString &msg) {
    if (type != QtDebugMsg && msg.contains(QLatin1String("qrc:/")) && !msg.contains(QLatin1String("deprecated"))) {
        g_qmlErrors++;
        fprintf(stderr, "FAIL qml: %s\n", qPrintable(msg));
    }
    if (g_previous) g_previous(type, ctx, msg);
}

int SmokeDriver::qmlErrors() { return g_qmlErrors; }

// -- WP QD38 -- Each section's time goes in the log, so growth shows before a
// budget is hit. A watchdog thread ends the run when one section runs longer
// than the limit and names it: a hang is found by its section, not by the
// whole run's timeout in run.sh. The limit is DSPI_SMOKE_SECTION_LIMIT seconds
// (0 turns it off).
namespace {
using Clock = std::chrono::steady_clock;
constexpr int kSectionLimitS = 120;
std::mutex g_sectionMutex;
std::string g_section = "start";
Clock::time_point g_sectionSince = Clock::now();
Clock::time_point g_runSince = Clock::now();
std::vector<std::pair<double, std::string>> g_sectionTimes;
std::atomic<int> g_sectionLimitS{0};

double secondsSince(Clock::time_point t) {
    return std::chrono::duration<double>(Clock::now() - t).count();
}

// Ends the current section; called with g_sectionMutex held
void closeSection() {
    const double s = secondsSince(g_sectionSince);
    g_sectionTimes.emplace_back(s, g_section);
    printf("smoke: -- %s: %.1f s\n", g_section.c_str(), s);
}

void startWatchdog(int limitS) {
    g_sectionLimitS = limitS;
    if (limitS <= 0) return;
    std::thread([]() {
        for (;;) {
            std::this_thread::sleep_for(std::chrono::seconds(1));
            std::lock_guard<std::mutex> lock(g_sectionMutex);
            const double s = secondsSince(g_sectionSince);
            if (s > g_sectionLimitS) {
                printf("FAIL watchdog: section \"%s\" still running after %.0f s (limit %d s, run %.0f s): a hang\n",
                       g_section.c_str(), s, g_sectionLimitS.load(), secondsSince(g_runSince));
                fflush(stdout);
                std::_Exit(4);
            }
        }
    }).detach();
}
} // namespace

void SmokeDriver::sectionTimes() {
    std::lock_guard<std::mutex> lock(g_sectionMutex);
    closeSection();
    g_section = "after the summary";
    g_sectionSince = Clock::now();
    auto slowest = g_sectionTimes;
    std::sort(slowest.begin(), slowest.end(), [](const auto &a, const auto &b) { return a.first > b.first; });
    if (slowest.size() > 5) slowest.resize(5);
    printf("smoke: %zu sections in %.1f s; slowest:", g_sectionTimes.size(), secondsSince(g_runSince));
    for (const auto &t : slowest) printf(" %s %.1f s;", t.second.c_str(), t.first);
    printf("\n");
    fflush(stdout);
}

// -- WP QP0 -- The sections, registered from their own files
namespace {
struct SectionEntry {
    const char *name;
    int order;
    SmokeDriver::SectionFn run;
};
std::vector<SectionEntry> &sections() {
    static std::vector<SectionEntry> list;
    return list;
}
} // namespace

SmokeDriver::Section::Section(const char *name, int order, SectionFn run) {
    sections().push_back({ name, order, run });
}

void SmokeDriver::start() {
    m_out = qEnvironmentVariable("DSPI_SMOKE_OUT", QDir::currentPath());
    g_previous = qInstallMessageHandler(messageHandler);
    // The hardware, replug and tray passes come back with QP19 and QP14
    for (const char *mode : { "DSPI_SMOKE_HW", "DSPI_SMOKE_TRAY", "DSPI_SMOKE_PSEUDO", "DSPI_SMOKE_DEMO_ENTRY" }) {
        if (!qEnvironmentVariableIsEmpty(mode)) {
            printf("smoke: %s is not in this build yet\n", mode);
            fflush(stdout);
            QTimer::singleShot(0, this, []() { QCoreApplication::exit(2); });
            return;
        }
    }
    // -- WP QD38 --
    bool limitSet = false;
    const int limit = qEnvironmentVariableIntValue("DSPI_SMOKE_SECTION_LIMIT", &limitSet);
    startWatchdog(limitSet ? limit : kSectionLimitS);
    QTimer::singleShot(200, this, [this]() { run(); });
}

void SmokeDriver::run() {
    std::vector<SectionEntry> list = sections();
    std::sort(list.begin(), list.end(), [](const SectionEntry &a, const SectionEntry &b) {
        if (a.order != b.order) return a.order < b.order;
        return std::string(a.name) < std::string(b.name);
    });
    const QStringList only = qEnvironmentVariable("DSPI_SMOKE_SECTION").split(QLatin1Char(','), Qt::SkipEmptyParts);
    int ran = 0;
    for (const SectionEntry &s : list) {
        if (!only.isEmpty() && !only.contains(QString::fromLatin1(s.name))) continue;
        section(s.name);
        s.run(*this);
        ++ran;
    }
    if (!only.isEmpty()) check(QStringLiteral("DSPI_SMOKE_SECTION names a section"), ran > 0);
    sectionTimes();
    m_failures += g_qmlErrors;
    printf("smoke: %d checks, %d failures (%d QML errors)\n", m_checks, m_failures, g_qmlErrors);
    fflush(stdout);
    QCoreApplication::exit(m_failures ? 1 : 0);
}

void SmokeDriver::section(const char *name) {
    // -- WP QD38 --
    std::lock_guard<std::mutex> lock(g_sectionMutex);
    closeSection();
    g_section = name;
    g_sectionSince = Clock::now();
    printf("smoke: == %s\n", name);
    fflush(stdout);
}

void SmokeDriver::check(const QString &what, bool ok) {
    m_checks++;
    if (!ok) {
        m_failures++;
        printf("FAIL %s\n", qPrintable(what));
        fflush(stdout);
    }
}

void SmokeDriver::spin(int ms) {
    QEventLoop loop;
    QTimer::singleShot(ms, &loop, &QEventLoop::quit);
    loop.exec();
}

bool SmokeDriver::waitFor(int ms, const std::function<bool()> &done) {
    QElapsedTimer t;
    t.start();
    while (!done()) {
        if (t.elapsed() > ms) return false;
        spin(20);
    }
    return true;
}

bool SmokeDriver::waitIdle(int ms) {
    // A few quiet turns of the event loop, so results already queued are applied
    return waitFor(ms, [this]() { return !m_.device->busy() && m_.core->worker()->pending() == 0; });
}

QQuickWindow *SmokeDriver::mainWindow() const {
    const QList<QObject *> roots = m_.engine->rootObjects();
    return roots.isEmpty() ? nullptr : qobject_cast<QQuickWindow *>(roots.first());
}

static QQuickItem *findItem(QQuickItem *scope, const QString &name) {
    if (!scope) return nullptr;
    if (scope->objectName() == name) return scope;
    for (QQuickItem *child : scope->childItems())
        if (QQuickItem *found = findItem(child, name)) return found;
    return nullptr;
}

QQuickItem *SmokeDriver::item(const QString &name, QQuickItem *scope) const {
    if (!scope) {
        QQuickWindow *w = mainWindow();
        if (!w) return nullptr;
        scope = w->contentItem();
    }
    return findItem(scope, name);
}

QObject *SmokeDriver::object(const QString &name) const {
    QQuickWindow *w = mainWindow();
    if (!w) return nullptr;
    if (QObject *o = w->findChild<QObject *>(name)) return o;
    return item(name);
}

bool SmokeDriver::grab(QQuickWindow *window, const QString &file) {
    if (!window) return false;
    const QImage image = window->grabWindow();
    if (image.isNull()) return false;
    return image.save(m_out + QLatin1Char('/') + file);
}

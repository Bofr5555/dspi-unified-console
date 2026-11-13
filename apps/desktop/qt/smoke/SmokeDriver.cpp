#include "SmokeDriver.h"

#include "Core.h"
#include "DeviceModel.h"

#include <QCoreApplication>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QImage>
#include <QtQml/QQmlApplicationEngine>
#include <QQuickItem>
#include <QQuickWindow>
#include <QThread>
#include <QTimer>
#include <algorithm>
#include <csignal>
#include <atomic>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <mutex>
#include <string>
#include <thread>
#include <utility>
#include <vector>
#if defined(__GLIBC__)
#include <execinfo.h>
#include <unistd.h>
#endif


// QML runtime errors fail the run (TypeError, ReferenceError, a broken binding)
static int g_qmlErrors = 0;
static QtMessageHandler g_previous = nullptr;
// -- WP QP19 -- A timer or an object used from a thread it does not live on
// fails every run (the qp0-core timer crash of review w2), not only a run that
// then crashes: Qt says so in a warning before anything breaks
static std::atomic<int> g_threadErrors{0};

static bool threadWarning(const QString &msg) {
    return msg.contains(QLatin1String("another thread")) || msg.contains(QLatin1String("different thread"))
        || msg.contains(QLatin1String("Timers can only be used with threads started with QThread"));
}

// -- WP QPI2 -- A crash or a cross-thread timer warning prints a backtrace and
// the thread, so a flaky run names the object and the thread that broke
// (review w2, the qp0-core timer crash). Symbols need -rdynamic to read well.
#if defined(__GLIBC__)
static void printBacktrace(const char *why) {
    char head[320];
    const QThread *t = QThread::currentThread();
    const int n = snprintf(head, sizeof head, "smoke: BACKTRACE %s on thread %p %s%s\n", why,
                           reinterpret_cast<void *>(QThread::currentThreadId()), t && t->isMainThread() ? "(main) " : "",
                           t ? qPrintable(t->objectName()) : "");
    if (n > 0) (void)!write(2, head, size_t(qMin(n, int(sizeof head) - 1)));
    void *frames[64];
    backtrace_symbols_fd(frames, backtrace(frames, 64), 2);
}
static void crashHandler(int sig) {
    printBacktrace(sig == SIGSEGV ? "SIGSEGV" : sig == SIGBUS ? "SIGBUS" : "SIGABRT");
    signal(sig, SIG_DFL);
    raise(sig);
}
#endif

static void messageHandler(QtMsgType type, const QMessageLogContext &ctx, const QString &msg) {
    if (threadWarning(msg)) {
        g_threadErrors++;
        fprintf(stderr, "FAIL thread: %s\n", qPrintable(msg));
#if defined(__GLIBC__)
        printBacktrace(qPrintable(msg));
#endif
    }
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
    SmokeDriver::Mode mode;
};
std::vector<SectionEntry> &sections() {
    static std::vector<SectionEntry> list;
    return list;
}
} // namespace

SmokeDriver::Section::Section(const char *name, int order, SectionFn run, Mode mode) {
    sections().push_back({ name, order, run, mode });
}

// -- WP QP19 -- The variables that choose a launch's mode, first match wins
namespace {
struct ModeVariable {
    const char *variable;
    SmokeDriver::Mode mode;
};
const ModeVariable kModeVariables[] = {
    { "DSPI_SMOKE_HW", SmokeDriver::Mode::Hardware },   // replug: below
    { "DSPI_SMOKE_DEMO_ENTRY", SmokeDriver::Mode::DemoEntry },
    { "DSPI_SMOKE_TRAY", SmokeDriver::Mode::Tray },
    { "DSPI_SMOKE_PSEUDO", SmokeDriver::Mode::Pseudo },
    { "DSPI_SMOKE_SELFTEST", SmokeDriver::Mode::SelfTest },
};
const char *modeVariable() {
    for (const ModeVariable &m : kModeVariables)
        if (!qEnvironmentVariableIsEmpty(m.variable)) return m.variable;
    return nullptr;
}
} // namespace

SmokeDriver::Mode SmokeDriver::mode() {
    const char *variable = modeVariable();
    if (!variable) return Mode::Main;
    for (const ModeVariable &m : kModeVariables) {
        if (qstrcmp(m.variable, variable) != 0) continue;
        if (m.mode == Mode::Hardware && qEnvironmentVariable(variable) == QLatin1String("replug")) return Mode::Replug;
        return m.mode;
    }
    return Mode::Main;
}

QString SmokeDriver::modeValue() {
    const char *variable = modeVariable();
    return variable ? qEnvironmentVariable(variable) : QString();
}

void SmokeDriver::start() {
    m_out = qEnvironmentVariable("DSPI_SMOKE_OUT", QDir::currentPath());
    g_previous = qInstallMessageHandler(messageHandler);
#if defined(__GLIBC__)
    for (int sig : { SIGSEGV, SIGBUS, SIGABRT }) signal(sig, crashHandler); // -- WP QPI2 --
#endif
    // -- WP QP19 -- A mode no section of this build runs ends the launch at once
    const Mode want = mode();
    if (std::none_of(sections().begin(), sections().end(), [want](const SectionEntry &s) { return s.mode == want; })) {
        printf("smoke: %s=%s has no section in this build\n", modeVariable(), qPrintable(modeValue()));
        fflush(stdout);
        QTimer::singleShot(0, this, []() { QCoreApplication::exit(2); });
        return;
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
    const Mode want = mode();
    for (const SectionEntry &s : list) {
        if (s.mode != want) continue;
        if (!only.isEmpty() && !only.contains(QString::fromLatin1(s.name))) continue;
        section(s.name);
        s.run(*this);
        ++ran;
    }
    if (!only.isEmpty()) check(QStringLiteral("DSPI_SMOKE_SECTION names a section"), ran > 0);
    if (m_quitExpected) return;   // the summary is printed; the app quits its own way
    summary();
    QCoreApplication::exit(m_failures ? 1 : 0);
}

void SmokeDriver::summary() {
    sectionTimes();
    m_failures += g_qmlErrors + g_threadErrors;
    const Mode m = mode();
    const QString notRun = m == Mode::Hardware || m == Mode::Replug ? QStringLiteral(", %1 not run").arg(m_notRun) : QString();
    printf("smoke: %d checks, %d failures (%d QML errors, %d thread warnings)%s\n", m_checks, m_failures, g_qmlErrors,
           g_threadErrors.load(), qPrintable(notRun));
    fflush(stdout);
}

void SmokeDriver::notRun(const QString &what, const QString &why) {
    m_notRun++;
    printf("smoke: NOT RUN %s: %s\n", qPrintable(what), qPrintable(why));
    fflush(stdout);
}

// -- WP QP19 -- behaviour.md §23: a tray launch that does not quit exits 3
void SmokeDriver::expectQuit(int withinMs) {
    m_quitExpected = true;
    summary();
    printf("smoke: tray launch %s: %d checks, %d failures, quitting\n", qPrintable(modeValue()), m_checks, m_failures);
    fflush(stdout);
    const bool failed = m_failures > 0;
    // The app's own quit decides the exit code, so a failure ends the launch here
    connect(qApp, &QCoreApplication::aboutToQuit, this, [failed]() {
        if (failed) std::_Exit(1);
    });
    std::thread([withinMs]() {
        std::this_thread::sleep_for(std::chrono::milliseconds(withinMs));
        printf("FAIL tray launch: the app did not quit within %d ms\n", withinMs);
        fflush(stdout);
        std::_Exit(3);
    }).detach();
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

#ifndef SETTINGSCONTROLLER_H
#define SETTINGSCONTROLLER_H

#include <QHash>
#include <QObject>
#include <QPointer>
#include <QStringList>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>
#include <initializer_list>
#include <vector>

#include "Registrar.h"

class PrefsMap;
class QQmlEngine;
class QJSEngine;

// The Settings window's pages, as the Mac's SettingsCategory lists them: a
// sidebar in four groups, each page with its title, icon and tint. Every work
// package that owns a page registers it here, in its own .cpp, at file scope:
//
//     static const SettingsRegistry::Page globalPage{
//         "global", SettingsRegistry::System, 50,
//         QT_TRANSLATE_NOOP("SettingsPages", "Global Parameters"), "hard-drive", "GlobalPage",
//         { QT_TRANSLATE_NOOP("SettingsPages", "Output latency"),
//           QT_TRANSLATE_NOOP("SettingsPages", "Startup preset"), ... } };
//
// - `id` is the page's lower-case id: what `settingsWindow/lastPage` stores and
//   what Theme.settingsTint(id) colours. The Mac's pages and their ids:
//       Application  about 10, updates 20, advanced 30, networking 40
//       Display      graphing 10, spectrum 20
//       System       overview 10, inputs 20, outputs 30, i2s 40, global 50
//       Control      surfaces 10, interfaces 20, groups 30, macros 40, aux 50
//   A page of group Hidden is not in the sidebar; another page opens it with
//   SettingsController.open(id) (the Core Voltage page, from Advanced).
// - `type` is the page's QML type in module UConsole; it is made when shown.
// - The search terms are what the search field matches besides the title:
//   every row's label on the page, and words people look for ("pin", "tray").
//   Translated through the context "SettingsPages". More can be added at run
//   time with SettingsController::addSearchTerms.
// - System and Control pages show only while a device is open. A page that the
//   open device lacks hides itself with setPageAvailable(id, false).
//
// A section another package adds to a page it does not own (the licence
// statement on About, the onboarding developer rows on Advanced):
//
//     static const SettingsRegistry::Section terms{ "about", 50, "AboutLicenceSection" };
//
// The page shows the registered sections in `order` at the place it keeps for
// them (SettingsExtensions). A bar along the bottom of the window on every page
// (the save bar):
//
//     static const SettingsRegistry::Footer saveBar{ 10, "SettingsSaveBar" };
namespace SettingsRegistry {

enum Group { Application, Display, System, Control, Hidden };

struct PageEntry {
    const char *id;
    Group group;
    int order;
    const char *title;
    const char *icon;
    const char *type;
    std::vector<const char *> terms;
};
struct SectionEntry {
    const char *page;
    int order;
    const char *type;
};
struct FooterEntry {
    int order;
    const char *type;
};

std::vector<PageEntry> &pages();
std::vector<SectionEntry> &sections();
std::vector<FooterEntry> &footers();

struct Page {
    Page(const char *id, Group group, int order, const char *title, const char *icon, const char *type,
         std::initializer_list<const char *> terms = {}) {
        pages().push_back({ id, group, order, title, icon, type, terms });
    }
};
struct Section {
    Section(const char *page, int order, const char *type) { sections().push_back({ page, order, type }); }
};
struct Footer {
    Footer(int order, const char *type) { footers().push_back({ order, type }); }
};

} // namespace SettingsRegistry

// The Settings window's controller: the page list and the search over it, the
// page history (back and forward, as the Mac's toolbar), the last page shown,
// the app preferences of the Display pages, and the device facts the
// Application and Overview pages read (pins, core voltage, analyser range).
class SettingsController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(QString currentPage READ currentPage NOTIFY currentPageChanged)
    Q_PROPERTY(QString currentTitle READ currentTitle NOTIFY currentPageChanged)
    Q_PROPERTY(QString currentType READ currentType NOTIFY currentPageChanged)
    Q_PROPERTY(bool canGoBack READ canGoBack NOTIFY currentPageChanged)
    Q_PROPERTY(bool canGoForward READ canGoForward NOTIFY currentPageChanged)
    // Bumped when the page list, a page's availability or its terms change
    Q_PROPERTY(int revision READ revision NOTIFY pagesChanged)
    Q_PROPERTY(QStringList footers READ footerTypes CONSTANT)
    // App preferences, applied at once (behaviour.md §8): graph/* and
    // spectrum/* in the settings file; QML binds `SettingsController.graph.lineWidth`
    Q_PROPERTY(PrefsMap *graph READ graph CONSTANT)
    Q_PROPERTY(PrefsMap *spectrum READ spectrum CONSTANT)
    Q_PROPERTY(bool showDebugInfo READ showDebugInfo WRITE setShowDebugInfo NOTIFY showDebugInfoChanged)
    // The app's update checker (QP17's AppUpdater), null until one is set; the
    // Updates page reads it (UpdatesPage.qml lists what it uses)
    Q_PROPERTY(QObject *updater READ updater NOTIFY updaterChanged)
    // Who holds each GPIO: [{pin, role, label}] in pin order (roles: 0 outputs,
    // 1 clocks, 2 inputs, 3 control, 4 other), and every pin the device offers
    Q_PROPERTY(QVariantList pinClaims READ pinClaims NOTIFY pinsChanged)
    Q_PROPERTY(QVariantList validPins READ validPins CONSTANT)
    // The core voltage extension: {supported, read, liveVreg, storedSel, ...}
    // (the core's field names); supported is false on firmware without it
    Q_PROPERTY(QVariantMap coreVoltage READ coreVoltage NOTIFY coreVoltageChanged)
    // The analyser's range: {supported, orderMin, orderMax, dynamicRangeDb}
    Q_PROPERTY(QVariantMap analyser READ analyser NOTIFY analyserChanged)
public:
    SettingsController(const AppContext &ctx, QObject *parent);
    ~SettingsController() override;
    static SettingsController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<SettingsController>(); }

    QString currentPage() const { return m_current; }
    QString currentTitle() const { return pageTitle(m_current); }
    QString currentType() const { return pageType(m_current); }
    bool canGoBack() const { return m_historyIndex > 0; }
    bool canGoForward() const { return m_historyIndex + 1 < m_history.size(); }
    int revision() const { return m_revision; }
    QStringList footerTypes() const;
    PrefsMap *graph() const { return m_graph; }
    PrefsMap *spectrum() const { return m_spectrum; }
    bool showDebugInfo() const { return m_showDebugInfo; }
    void setShowDebugInfo(bool on);
    QObject *updater() const { return m_updater; }
    QVariantList pinClaims() const { return m_pinClaims; }
    QVariantList validPins() const;
    QVariantMap coreVoltage() const { return m_coreVoltage; }
    QVariantMap analyser() const { return m_analyser; }

    // For the package that owns a page: hide it while the open device lacks it
    void setPageAvailable(const QString &id, bool available);
    void addSearchTerms(const QString &id, const QStringList &terms);
    void setUpdater(QObject *updater);

    // The sidebar for `query`: [{heading}] and [{id, title, icon}] rows in
    // order, only pages that match and are shown now; [] when nothing matches.
    Q_INVOKABLE QVariantList sidebarRows(const QString &query) const;
    // Whether page `id` matches `query` (title or search terms, ignoring case)
    Q_INVOKABLE bool matches(const QString &id, const QString &query) const;
    Q_INVOKABLE bool isShown(const QString &id) const;
    Q_INVOKABLE QString pageTitle(const QString &id) const;
    Q_INVOKABLE QString pageType(const QString &id) const;
    Q_INVOKABLE QString pageIcon(const QString &id) const;
    // The QML types other packages registered for page `id`, in order
    Q_INVOKABLE QStringList sectionTypes(const QString &id) const;

    // A fresh navigation: drops forward entries and records it
    Q_INVOKABLE void open(const QString &id);
    Q_INVOKABLE void back();
    Q_INVOKABLE void forward();
    // Opens the Settings window on page `id`
    Q_INVOKABLE void showPage(const QString &id);
    // When the window opens: a page the device does not have falls back to About
    Q_INVOKABLE void ensureShown();

    // Advanced: every channel's name back to the device's default, at once
    Q_INVOKABLE void resetChannelNames();
    // Overview: read the core voltage status again (the refresh button)
    Q_INVOKABLE void refreshCoreVoltage();
    // Re-reads what the Overview and Spectrum pages show (their onVisible)
    Q_INVOKABLE void refreshDeviceFacts();
    // "1.20 V" for VREG code `code` on the open platform, "code 0x1F" when it
    // does not decode (the Mac's VregVoltage.label)
    Q_INVOKABLE QString voltsLabel(int code) const;
    // "3 h 12 min", "4 min 5 s", "42 s" (the Mac's formatUptime)
    Q_INVOKABLE QString uptimeLabel(qint64 seconds) const;

signals:
    void currentPageChanged();
    void pagesChanged();
    void showDebugInfoChanged();
    void updaterChanged();
    void pinsChanged();
    void coreVoltageChanged();
    void analyserChanged();

private:
    const SettingsRegistry::PageEntry *entry(const QString &id) const;
    void navigate(const QString &id, bool record);
    void onSnapshot();
    void readPins();

    AppContext m_ctx;
    PrefsMap *m_graph;
    PrefsMap *m_spectrum;
    QString m_current;
    QStringList m_history;
    int m_historyIndex = 0;
    int m_revision = 0;
    QHash<QString, bool> m_unavailable;
    QHash<QString, QStringList> m_extraTerms;
    bool m_showDebugInfo = false;
    QPointer<QObject> m_updater;
    QVariantList m_pinClaims;
    QVariantMap m_coreVoltage;
    QVariantMap m_analyser;
    uint64_t m_factsGeneration = 0;
    bool m_connected = false;
};

#endif // SETTINGSCONTROLLER_H

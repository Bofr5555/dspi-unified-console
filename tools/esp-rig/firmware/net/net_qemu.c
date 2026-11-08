/*
 * net_qemu.c - the bridge's net.h over QEMU's OpenCores Ethernet MAC, for
 * the LK3 emulator rig only. Replaces upstream's net.c (which drives the
 * Wi-Fi radio QEMU does not emulate); everything above lwIP is upstream's.
 *
 * The bridge comes up as a station already joined to its home network: a
 * fixed IPv4 address on the emulated Ethernet, mDNS with the same _dspi._tcp
 * advertisement net.c makes, and no setup network. The Wi-Fi-only calls
 * (scan, join, finish setup) answer as a bridge with no radio would: no
 * networks, refused.
 *
 * The address comes from the build (RIG_IP, RIG_NETMASK, RIG_GATEWAY,
 * set by tools/esp-rig/firmware/build.sh), default 10.77.0.2/24 via
 * 10.77.0.1.
 *
 * net_advertise, is_hex_key and net_check_credentials are copied from
 * upstream's components/net/net.c (WeebLabs/DSPi-Link bridge/esp32, at the
 * conformance pin), under its MIT licence:
 *
 *   Copyright (c) 2026 Weeb Labs
 *
 *   Permission is hereby granted, free of charge, to any person obtaining a
 *   copy of this software and associated documentation files (the
 *   "Software"), to deal in the Software without restriction, including
 *   without limitation the rights to use, copy, modify, merge, publish,
 *   distribute, sublicense, and/or sell copies of the Software, and to
 *   permit persons to whom the Software is furnished to do so, subject to
 *   the following conditions: the above copyright notice and this
 *   permission notice shall be included in all copies or substantial
 *   portions of the Software. The full text: LICENSE.DSPi-Link next to
 *   this file.
 */

#include "net.h"

#include <ctype.h>
#include <stdio.h>
#include <string.h>

#include "bridge_config.h"
#include "esp_check.h"
#include "esp_eth.h"
#include "esp_eth_mac_openeth.h"
#include "esp_event.h"
#include "esp_log.h"
#include "esp_mac.h"
#include "esp_netif.h"
#include "freertos/FreeRTOS.h"
#include "lwip/ip4_addr.h"
#include "mdns.h"
#include "settings.h"

#ifndef RIG_IP
#define RIG_IP "10.77.0.2"
#endif
#ifndef RIG_NETMASK
#define RIG_NETMASK "255.255.255.0"
#endif
#ifndef RIG_GATEWAY
#define RIG_GATEWAY "10.77.0.1"
#endif

static const char *TAG = "net";

#define SERVICE_TYPE "_dspi"
#define SERVICE_PROTOCOL "_tcp"
#define RIG_SSID "LK3 rig (Ethernet)"

static struct {
    esp_netif_t *netif;
    esp_eth_handle_t eth;
    bool service_added; /* Hub task only */
    net_status_t status;
    portMUX_TYPE status_lock;
} net_state = {.status_lock = portMUX_INITIALIZER_UNLOCKED};

static void publish(bool connected, const char *ip)
{
    taskENTER_CRITICAL(&net_state.status_lock);
    net_status_t *status = &net_state.status;
    status->state = connected ? NET_STATE_ONLINE : NET_STATE_CONNECTING;
    status->setup_network_on = false;
    status->join = connected ? NET_JOIN_CONNECTED : NET_JOIN_CONNECTING;
    status->join_failure = NET_JOIN_FAILURE_NONE;
    status->connected = connected;
    snprintf(status->ip, sizeof(status->ip), "%s", connected ? ip : "");
    snprintf(status->ssid, sizeof(status->ssid), "%s", connected ? RIG_SSID : "");
    snprintf(status->saved_ssid, sizeof(status->saved_ssid), "%s", RIG_SSID);
    status->rssi = connected ? -50 : 0;
    taskEXIT_CRITICAL(&net_state.status_lock);
}

static void on_event(void *argument, esp_event_base_t base, int32_t id, void *data)
{
    (void)argument;
    if (base == ETH_EVENT && id == ETHERNET_EVENT_CONNECTED) {
        ESP_LOGI(TAG, "Ethernet link up");
    } else if (base == ETH_EVENT && id == ETHERNET_EVENT_DISCONNECTED) {
        ESP_LOGW(TAG, "Ethernet link down");
        publish(false, "");
    } else if (base == IP_EVENT && id == IP_EVENT_ETH_GOT_IP) {
        const ip_event_got_ip_t *event = data;
        char ip[16];
        esp_ip4addr_ntoa(&event->ip_info.ip, ip, sizeof(ip));
        ESP_LOGI(TAG, "Online at %s (emulated Ethernet, LK3 rig)", ip);
        publish(true, ip);
    }
}

esp_err_t net_set_hostname(const char *hostname)
{
    ESP_RETURN_ON_ERROR(esp_netif_set_hostname(net_state.netif, hostname), TAG,
                        "setting the DHCP hostname");
    taskENTER_CRITICAL(&net_state.status_lock);
    snprintf(net_state.status.hostname, sizeof(net_state.status.hostname), "%s", hostname);
    taskEXIT_CRITICAL(&net_state.status_lock);
    return mdns_hostname_set(hostname);
}

void net_get_status(net_status_t *status)
{
    taskENTER_CRITICAL(&net_state.status_lock);
    *status = net_state.status;
    taskEXIT_CRITICAL(&net_state.status_lock);
}

/* As net.c's net_advertise, unchanged. */
esp_err_t net_advertise(const char *instance_name, const net_txt_entry_t *txt, size_t count)
{
    mdns_txt_item_t items[NET_MAX_TXT_ENTRIES];
    if (count > NET_MAX_TXT_ENTRIES) {
        return ESP_ERR_INVALID_ARG;
    }
    for (size_t i = 0; i < count; i++) {
        items[i] = (mdns_txt_item_t){.key = txt[i].key, .value = txt[i].value};
    }
    if (!net_state.service_added) {
        ESP_RETURN_ON_ERROR(mdns_service_add(instance_name, SERVICE_TYPE, SERVICE_PROTOCOL,
                                             LINK_TCP_PORT, items, count),
                            TAG, "adding the DNS-SD service");
        net_state.service_added = true;
        return ESP_OK;
    }
    ESP_RETURN_ON_ERROR(
        mdns_service_instance_name_set(SERVICE_TYPE, SERVICE_PROTOCOL, instance_name), TAG,
        "renaming the DNS-SD service");
    return mdns_service_txt_set(SERVICE_TYPE, SERVICE_PROTOCOL, items, count);
}

esp_err_t net_scan_networks(net_network_t *networks, size_t capacity, size_t *found)
{
    (void)networks;
    (void)capacity;
    *found = 0; /* No radio: no networks in range */
    return ESP_OK;
}

size_t net_scan(net_network_t *networks, size_t capacity)
{
    size_t found = 0;
    (void)net_scan_networks(networks, capacity, &found);
    return found;
}

static bool is_hex_key(const char *text)
{
    for (const char *c = text; *c != '\0'; c++) {
        if (!isxdigit((unsigned char)*c)) {
            return false;
        }
    }
    return true;
}

/* As net.c's check, unchanged, so the setup API refuses the same input. */
net_credentials_check_t net_check_credentials(const char *ssid, const char *password)
{
    const size_t ssid_length = strlen(ssid);
    if (ssid_length == 0 || ssid_length > SETTINGS_WIFI_SSID_MAX_BYTES) {
        return NET_CREDENTIALS_BAD_SSID;
    }
    const size_t length = strlen(password);
    const bool passphrase = length >= 8 && length <= 63;
    const bool raw_key = length == 64 && is_hex_key(password);
    return length == 0 || passphrase || raw_key ? NET_CREDENTIALS_OK : NET_CREDENTIALS_BAD_PASSWORD;
}

esp_err_t net_join(const char *ssid, const char *password)
{
    if (net_check_credentials(ssid, password) != NET_CREDENTIALS_OK) {
        return ESP_ERR_INVALID_ARG;
    }
    ESP_LOGW(TAG, "Join of \"%s\" ignored: the LK3 rig has no radio", ssid);
    return ESP_ERR_NOT_SUPPORTED;
}

esp_err_t net_finish_setup(void)
{
    return ESP_OK; /* There is no setup network to switch off */
}

bool net_socket_is_on_setup_network(int fd)
{
    (void)fd;
    return false;
}

const char *net_setup_address(void)
{
    return "192.168.4.1";
}

static esp_err_t start_ethernet(void)
{
    eth_mac_config_t mac_config = ETH_MAC_DEFAULT_CONFIG();
    eth_phy_config_t phy_config = ETH_PHY_DEFAULT_CONFIG();
    phy_config.autonego_timeout_ms = 100;
    esp_eth_mac_t *mac = esp_eth_mac_new_openeth(&mac_config);
    esp_eth_phy_t *phy = esp_eth_phy_new_generic(&phy_config);
    ESP_RETURN_ON_FALSE(mac != NULL && phy != NULL, ESP_ERR_NO_MEM, TAG, "Ethernet MAC/PHY");
    esp_eth_config_t config = ETH_DEFAULT_CONFIG(mac, phy);
    ESP_RETURN_ON_ERROR(esp_eth_driver_install(&config, &net_state.eth), TAG, "Ethernet driver");

    /* The station's MAC, so the hostname digits match a real bridge's. */
    uint8_t mac_address[6];
    ESP_RETURN_ON_ERROR(esp_read_mac(mac_address, ESP_MAC_WIFI_STA), TAG, "reading the MAC");
    ESP_RETURN_ON_ERROR(esp_eth_ioctl(net_state.eth, ETH_CMD_S_MAC_ADDR, mac_address), TAG,
                        "setting the MAC");
    snprintf(net_state.status.mac, sizeof(net_state.status.mac), "%02x:%02x:%02x:%02x:%02x:%02x",
             mac_address[0], mac_address[1], mac_address[2], mac_address[3], mac_address[4],
             mac_address[5]);
    snprintf(net_state.status.setup_ssid, sizeof(net_state.status.setup_ssid),
             "DSPi Bridge %02X%02X", mac_address[4], mac_address[5]);

    ESP_RETURN_ON_ERROR(esp_netif_dhcpc_stop(net_state.netif), TAG, "stopping DHCP");
    esp_netif_ip_info_t ip = {0};
    ip.ip.addr = ipaddr_addr(RIG_IP);
    ip.netmask.addr = ipaddr_addr(RIG_NETMASK);
    ip.gw.addr = ipaddr_addr(RIG_GATEWAY);
    ESP_RETURN_ON_ERROR(esp_netif_set_ip_info(net_state.netif, &ip), TAG, "the fixed address");

    ESP_RETURN_ON_ERROR(
        esp_netif_attach(net_state.netif, esp_eth_new_netif_glue(net_state.eth)), TAG,
        "attaching Ethernet to the stack");
    return esp_eth_start(net_state.eth);
}

esp_err_t net_start(const char *hostname)
{
    snprintf(net_state.status.hostname, sizeof(net_state.status.hostname), "%s", hostname);
    publish(false, "");
    ESP_RETURN_ON_ERROR(esp_netif_init(), TAG, "starting the network stack");
    ESP_RETURN_ON_ERROR(esp_event_loop_create_default(), TAG, "starting the event loop");
    const esp_netif_config_t netif_config = ESP_NETIF_DEFAULT_ETH();
    net_state.netif = esp_netif_new(&netif_config);
    ESP_RETURN_ON_FALSE(net_state.netif != NULL, ESP_ERR_NO_MEM, TAG, "the network interface");
    ESP_RETURN_ON_ERROR(esp_netif_set_hostname(net_state.netif, hostname), TAG,
                        "setting the name DHCP reports");
    ESP_RETURN_ON_ERROR(esp_event_handler_register(ETH_EVENT, ESP_EVENT_ANY_ID, on_event, NULL),
                        TAG, "Ethernet events");
    ESP_RETURN_ON_ERROR(esp_event_handler_register(IP_EVENT, IP_EVENT_ETH_GOT_IP, on_event, NULL),
                        TAG, "IP events");
    ESP_RETURN_ON_ERROR(start_ethernet(), TAG, "starting Ethernet");
    ESP_RETURN_ON_ERROR(mdns_init(), TAG, "starting mDNS");
    ESP_RETURN_ON_ERROR(mdns_hostname_set(hostname), TAG, "setting the mDNS hostname");
    ESP_LOGW(TAG, "LK3 rig build: Wi-Fi replaced by emulated Ethernet at %s", RIG_IP);
    return ESP_OK;
}

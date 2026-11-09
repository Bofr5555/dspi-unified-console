# net, LK3 rig build: upstream's portable net_machine.c and captive_dns.c with
# net_qemu.c (emulated Ethernet) in place of net.c (the Wi-Fi radio). The
# rig's build.sh copies this file and net_qemu.c over upstream's component.
idf_component_register(
    SRCS "net_qemu.c" "net_machine.c" "captive_dns.c"
    INCLUDE_DIRS "include"
    REQUIRES esp_common
    PRIV_REQUIRES bridge_config esp_eth esp_netif esp_event esp_timer freertos log lwip settings)

target_compile_options(${COMPONENT_LIB} PRIVATE -Wextra -Werror)
if(DEFINED ENV{RIG_IP})
    target_compile_definitions(${COMPONENT_LIB} PRIVATE RIG_IP="$ENV{RIG_IP}")
endif()

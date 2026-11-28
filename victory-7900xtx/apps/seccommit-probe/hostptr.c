// Why does vkd3d-proton's host-memory import fail under Wine for a section and
// work for private memory? This asks the driver itself, on this Linux side and
// with no Wine in the way: VK_EXT_external_memory_host on four kinds of pages.
//
//   gcc -O2 -o hostptr hostptr.c -lvulkan && MESA_VK_DEVICE_SELECT=1002:747e ./hostptr
//
// The four: anonymous private pages (what VirtualAlloc gives: the path that
// works), a memfd (what Wine's pagefile-backed sections are, and what its
// shm-backed section views are), a tmpfs file's mapping, and an ext4 file's
// mapping (page cache).
#define _GNU_SOURCE
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <vulkan/vulkan.h>

#define SIZE (2u << 20)

static const char *vr(VkResult r) {
    switch (r) {
    case VK_SUCCESS: return "VK_SUCCESS";
    case VK_ERROR_OUT_OF_HOST_MEMORY: return "VK_ERROR_OUT_OF_HOST_MEMORY";
    case VK_ERROR_OUT_OF_DEVICE_MEMORY: return "VK_ERROR_OUT_OF_DEVICE_MEMORY";
    case VK_ERROR_INVALID_EXTERNAL_HANDLE: return "VK_ERROR_INVALID_EXTERNAL_HANDLE";
    case VK_ERROR_FEATURE_NOT_PRESENT: return "VK_ERROR_FEATURE_NOT_PRESENT";
    case VK_ERROR_INITIALIZATION_FAILED: return "VK_ERROR_INITIALIZATION_FAILED";
    case VK_ERROR_MEMORY_MAP_FAILED: return "VK_ERROR_MEMORY_MAP_FAILED";
    case VK_ERROR_EXTENSION_NOT_PRESENT: return "VK_ERROR_EXTENSION_NOT_PRESENT";
    default: return "VK_ERROR_OTHER";
    }
}

typedef struct {
    VkPhysicalDevice phy;
    VkDevice dev;
    VkPhysicalDeviceMemoryProperties mem;
    uint32_t types;      // memoryTypeBits the driver allows for host pointers
    uint64_t alignment;
} Ctx;

static PFN_vkGetMemoryHostPointerPropertiesEXT p_GetProps;

static int try_import(Ctx *c, const char *name, void *p) {
    VkMemoryHostPointerPropertiesEXT hp = { .sType = VK_STRUCTURE_TYPE_MEMORY_HOST_POINTER_PROPERTIES_EXT };
    VkResult r = p_GetProps(c->dev,
        VK_EXTERNAL_MEMORY_HANDLE_TYPE_HOST_ALLOCATION_BIT_EXT, p, &hp);
    if (r != VK_SUCCESS) {
        printf("%-34s pointer %p: vkGetMemoryHostPointerPropertiesEXT %s\n", name, p, vr(r));
        return 0;
    }
    uint32_t type = UINT32_MAX;
    for (uint32_t i = 0; i < c->mem.memoryTypeCount; i++)
        if ((hp.memoryTypeBits & (1u << i)) && (c->mem.memoryTypes[i].propertyFlags & VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT)) {
            type = i;
            break;
        }
    if (type == UINT32_MAX) {
        printf("%-34s pointer %p: the driver allows host pointers on no host-visible type (types %#x)\n", name, p, hp.memoryTypeBits);
        return 0;
    }
    VkMemoryPropertyFlags flags = c->mem.memoryTypes[type].propertyFlags;
    VkImportMemoryHostPointerInfoEXT info = {
        .sType = VK_STRUCTURE_TYPE_IMPORT_MEMORY_HOST_POINTER_INFO_EXT,
        .handleType = VK_EXTERNAL_MEMORY_HANDLE_TYPE_HOST_ALLOCATION_BIT_EXT,
        .pHostPointer = p,
    };
    VkMemoryAllocateInfo ai = {
        .sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        .pNext = &info,
        .allocationSize = SIZE,
        .memoryTypeIndex = type,
    };
    VkDeviceMemory m = VK_NULL_HANDLE;
    r = vkAllocateMemory(c->dev, &ai, NULL, &m);
    printf("%-34s pointer %p: type %u (props %#x, %s%s%s): %s\n", name, p, type, flags,
        (flags & VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT) ? "device-local " : "",
        (flags & VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT) ? "host-visible " : "",
        (flags & VK_MEMORY_PROPERTY_HOST_COHERENT_BIT) ? "host-coherent" : "",
        vr(r));
    if (r == VK_SUCCESS) {
        // What the driver makes of the pages once imported: read them back
        // through its own mapping, and write through it.
        void *q = NULL;
        if (vkMapMemory(c->dev, m, 0, SIZE, 0, &q) == VK_SUCCESS) {
            unsigned char want = *(volatile unsigned char *)p;
            unsigned char got = *(volatile unsigned char *)q;
            printf("%-34s   the driver's mapping sees the CPU's first byte %s (0x%02x vs 0x%02x)\n",
                name, want == got ? "as it is" : "differently", want, got);
            vkUnmapMemory(c->dev, m);
        }
        vkFreeMemory(c->dev, m, NULL);
    }
    return r == VK_SUCCESS;
}

int main(void) {
    VkApplicationInfo app = { .sType = VK_STRUCTURE_TYPE_APPLICATION_INFO, .apiVersion = VK_API_VERSION_1_3 };
    const char *ext[] = { "VK_EXT_external_memory_host" };
    VkInstanceCreateInfo ici = { .sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO, .pApplicationInfo = &app,
        .enabledExtensionCount = 0, .ppEnabledExtensionNames = NULL };
    VkInstance inst;
    if (vkCreateInstance(&ici, NULL, &inst) != VK_SUCCESS) { printf("no instance\n"); return 1; }
    uint32_t n = 0;
    vkEnumeratePhysicalDevices(inst, &n, NULL);
    VkPhysicalDevice phys[8];
    if (n > 8) n = 8;
    vkEnumeratePhysicalDevices(inst, &n, phys);
    Ctx c = {0};
    for (uint32_t i = 0; i < n; i++) {
        VkPhysicalDeviceProperties p;
        vkGetPhysicalDeviceProperties(phys[i], &p);
        printf("device %u: %s\n", i, p.deviceName);
        if (p.deviceID == 0x747e) c.phy = phys[i];
    }
    if (!c.phy) { printf("no 0x747e device\n"); return 1; }
    VkPhysicalDeviceExternalMemoryHostPropertiesEXT host = { .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_MEMORY_HOST_PROPERTIES_EXT };
    VkPhysicalDeviceProperties2 p2 = { .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2, .pNext = &host };
    vkGetPhysicalDeviceProperties2(c.phy, &p2);
    printf("minImportedHostPointerAlignment: %llu\n", (unsigned long long)host.minImportedHostPointerAlignment);
    c.alignment = host.minImportedHostPointerAlignment;
    vkGetPhysicalDeviceMemoryProperties(c.phy, &c.mem);
    float prio = 1.0f;
    VkDeviceQueueCreateInfo q = { .sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO, .queueCount = 1, .pQueuePriorities = &prio };
    VkDeviceCreateInfo dci = { .sType = VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO, .queueCreateInfoCount = 1,
        .pQueueCreateInfos = &q, .enabledExtensionCount = 1, .ppEnabledExtensionNames = ext };
    if (vkCreateDevice(c.phy, &dci, NULL, &c.dev) != VK_SUCCESS) { printf("no device\n"); return 1; }
    p_GetProps = (PFN_vkGetMemoryHostPointerPropertiesEXT)vkGetDeviceProcAddr(c.dev, "vkGetMemoryHostPointerPropertiesEXT");
    if (!p_GetProps) { printf("no vkGetMemoryHostPointerPropertiesEXT\n"); return 1; }

    // 1. anonymous private pages, touched.
    void *anon = mmap(NULL, SIZE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    memset(anon, 0xa5, SIZE);
    try_import(&c, "anonymous private (VirtualAlloc)", anon);

    // 2. a memfd, mapped shared: what Wine's pagefile sections are.
    int fd = memfd_create("section", 0);
    if (ftruncate(fd, SIZE) == 0) {
        void *mf = mmap(NULL, SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
        memset(mf, 0xa5, SIZE);
        try_import(&c, "memfd section (SEC_COMMIT)", mf);
    }

    // 3. a tmpfs file, mapped shared.
    int tf = open("/dev/shm/hostptr-test", O_RDWR | O_CREAT | O_TRUNC, 0600);
    if (tf >= 0 && ftruncate(tf, SIZE) == 0) {
        void *tm = mmap(NULL, SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, tf, 0);
        memset(tm, 0xa5, SIZE);
        try_import(&c, "tmpfs file (backed section)", tm);
        unlink("/dev/shm/hostptr-test");
    }

    // 4. a file in the home directory's filesystem (page cache).
    const char *path = "/home/manse/.hermes/cache/scratch/hostptr-file";
    int hf = open(path, O_RDWR | O_CREAT | O_TRUNC, 0600);
    if (hf >= 0 && ftruncate(hf, SIZE) == 0) {
        void *hm = mmap(NULL, SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, hf, 0);
        memset(hm, 0xa5, SIZE);
        try_import(&c, "disk file (page cache)", hm);
        unlink(path);
    }

    // 5. the same private pages, made read-only under the import's feet? No:
    //    whether a private mapping that is write-watched (Wine's guards) still
    //    imports is what port 1 relies on; it is the same anonymous mapping.
    printf("done\n");
    return 0;
}

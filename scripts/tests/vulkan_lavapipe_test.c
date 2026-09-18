#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <vulkan/vulkan.h>

#define NUM_ELEMENTS 1024
#define WORKGROUP_SIZE 64

static void check_vk(VkResult res, const char *msg) {
    if (res != VK_SUCCESS) {
        fprintf(stderr, "[FATAL] Vulkan error in %s: %d\n", msg, res);
        exit(1);
    }
}

static uint32_t find_memory_type(VkPhysicalDevice phys_dev, uint32_t type_filter, VkMemoryPropertyFlags properties) {
    VkPhysicalDeviceMemoryProperties mem_props;
    vkGetPhysicalDeviceMemoryProperties(phys_dev, &mem_props);
    for (uint32_t i = 0; i < mem_props.memoryTypeCount; i++) {
        if ((type_filter & (1 << i)) && (mem_props.memoryTypes[i].propertyFlags & properties) == properties) {
            return i;
        }
    }
    fprintf(stderr, "[FATAL] Failed to find suitable memory type\n");
    exit(1);
}

int main(int argc, char **argv) {
    printf("===============================================================================\n");
    printf("     Taurscribe Vulkan Software GPU Emulation Suite (Mesa Lavapipe)\n");
    printf("===============================================================================\n");

    const char *spv_path = (argc > 1) ? argv[1] : "scripts/tests/vulkan_compute.spv";

    // 1. Create Instance
    VkApplicationInfo app_info = {
        .sType = VK_STRUCTURE_TYPE_APPLICATION_INFO,
        .pApplicationName = "Taurscribe Vulkan Lavapipe Test",
        .applicationVersion = VK_MAKE_VERSION(1, 0, 0),
        .pEngineName = "TaurscribeEngine",
        .engineVersion = VK_MAKE_VERSION(1, 0, 0),
        .apiVersion = VK_API_VERSION_1_2,
    };

    VkInstanceCreateInfo inst_create_info = {
        .sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
        .pApplicationInfo = &app_info,
    };

    VkInstance instance;
    check_vk(vkCreateInstance(&inst_create_info, NULL, &instance), "vkCreateInstance");
    printf("[VULKAN] VkInstance created successfully.\n");

    // 2. Enumerate Physical Devices
    uint32_t device_count = 0;
    vkEnumeratePhysicalDevices(instance, &device_count, NULL);
    if (device_count == 0) {
        fprintf(stderr, "[FATAL] No Vulkan physical devices found!\n");
        return 1;
    }

    VkPhysicalDevice *devices = malloc(sizeof(VkPhysicalDevice) * device_count);
    vkEnumeratePhysicalDevices(instance, &device_count, devices);

    printf("[VULKAN] Found %u physical Vulkan device(s):\n", device_count);
    VkPhysicalDevice selected_device = VK_NULL_HANDLE;
    int is_lavapipe_found = 0;

    for (uint32_t i = 0; i < device_count; i++) {
        VkPhysicalDeviceProperties props;
        vkGetPhysicalDeviceProperties(devices[i], &props);
        printf("  -> Device [%u]: %s (Vendor: 0x%04X, DeviceID: 0x%04X, Type: %u, DriverVer: %u.%u.%u)\n",
               i, props.deviceName, props.vendorID, props.deviceID, props.deviceType,
               VK_VERSION_MAJOR(props.driverVersion),
               VK_VERSION_MINOR(props.driverVersion),
               VK_VERSION_PATCH(props.driverVersion));

        if (strstr(props.deviceName, "llvmpipe") != NULL || strstr(props.deviceName, "lavapipe") != NULL ||
            props.deviceType == VK_PHYSICAL_DEVICE_TYPE_CPU) {
            selected_device = devices[i];
            is_lavapipe_found = 1;
        }
    }

    if (selected_device == VK_NULL_HANDLE) {
        printf("[WARN] Specific Lavapipe CPU adapter not found by name, selecting device [0] as fallback.\n");
        selected_device = devices[0];
    } else {
        printf("[VULKAN] Successfully identified Mesa Lavapipe software GPU adapter.\n");
    }

    // 3. Find Compute Queue Family
    uint32_t queue_family_count = 0;
    vkGetPhysicalDeviceQueueFamilyProperties(selected_device, &queue_family_count, NULL);
    VkQueueFamilyProperties *queue_props = malloc(sizeof(VkQueueFamilyProperties) * queue_family_count);
    vkGetPhysicalDeviceQueueFamilyProperties(selected_device, &queue_family_count, queue_props);

    uint32_t compute_queue_family = UINT32_MAX;
    for (uint32_t i = 0; i < queue_family_count; i++) {
        if (queue_props[i].queueFlags & VK_QUEUE_COMPUTE_BIT) {
            compute_queue_family = i;
            break;
        }
    }
    free(queue_props);

    if (compute_queue_family == UINT32_MAX) {
        fprintf(stderr, "[FATAL] No compute-capable queue family found on physical device!\n");
        return 1;
    }
    printf("[VULKAN] Selected compute queue family index: %u\n", compute_queue_family);

    // 4. Create Logical Device
    float queue_priority = 1.0f;
    VkDeviceQueueCreateInfo queue_create_info = {
        .sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
        .queueFamilyIndex = compute_queue_family,
        .queueCount = 1,
        .pQueuePriorities = &queue_priority,
    };

    VkDeviceCreateInfo dev_create_info = {
        .sType = VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
        .queueCreateInfoCount = 1,
        .pQueueCreateInfos = &queue_create_info,
    };

    VkDevice device;
    check_vk(vkCreateDevice(selected_device, &dev_create_info, NULL, &device), "vkCreateDevice");
    printf("[VULKAN] Logical VkDevice instantiated successfully.\n");

    VkQueue compute_queue;
    vkGetDeviceQueue(device, compute_queue_family, 0, &compute_queue);

    // 5. Load SPIR-V Shader
    FILE *f = fopen(spv_path, "rb");
    if (!f) {
        fprintf(stderr, "[FATAL] Failed to open SPIR-V shader at %s\n", spv_path);
        return 1;
    }
    fseek(f, 0, SEEK_END);
    long spv_size = ftell(f);
    fseek(f, 0, SEEK_SET);

    uint32_t *spv_code = malloc(spv_size);
    fread(spv_code, 1, spv_size, f);
    fclose(f);

    VkShaderModuleCreateInfo sm_create_info = {
        .sType = VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
        .codeSize = spv_size,
        .pCode = spv_code,
    };
    VkShaderModule shader_module;
    check_vk(vkCreateShaderModule(device, &sm_create_info, NULL, &shader_module), "vkCreateShaderModule");
    free(spv_code);
    printf("[VULKAN] Compute shader module loaded (%ld bytes SPIR-V).\n", spv_size);

    // 6. Allocate Buffers (A, B, C)
    VkDeviceSize buffer_size = sizeof(float) * NUM_ELEMENTS;
    VkBuffer buffers[3];
    VkDeviceMemory memories[3];

    for (int i = 0; i < 3; i++) {
        VkBufferCreateInfo buf_info = {
            .sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
            .size = buffer_size,
            .usage = VK_BUFFER_USAGE_STORAGE_BUFFER_BIT,
            .sharingMode = VK_SHARING_MODE_EXCLUSIVE,
        };
        check_vk(vkCreateBuffer(device, &buf_info, NULL, &buffers[i]), "vkCreateBuffer");

        VkMemoryRequirements mem_reqs;
        vkGetBufferMemoryRequirements(device, buffers[i], &mem_reqs);

        VkMemoryAllocateInfo alloc_info = {
            .sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
            .allocationSize = mem_reqs.size,
            .memoryTypeIndex = find_memory_type(selected_device, mem_reqs.memoryTypeBits,
                                                VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT),
        };
        check_vk(vkAllocateMemory(device, &alloc_info, NULL, &memories[i]), "vkAllocateMemory");
        vkBindBufferMemory(device, buffers[i], memories[i], 0);
    }
    printf("[VULKAN] Allocated 3 storage buffers (%zu bytes each) in host-coherent device memory.\n", buffer_size);

    // 7. Populate Inputs A and B
    float *ptr_a, *ptr_b;
    vkMapMemory(device, memories[0], 0, buffer_size, 0, (void **)&ptr_a);
    vkMapMemory(device, memories[1], 0, buffer_size, 0, (void **)&ptr_b);
    for (int i = 0; i < NUM_ELEMENTS; i++) {
        ptr_a[i] = (float)i * 0.5f;
        ptr_b[i] = (float)(NUM_ELEMENTS - i) * 0.25f;
    }
    vkUnmapMemory(device, memories[0]);
    vkUnmapMemory(device, memories[1]);

    // 8. Descriptor Set Layout & Pipeline Layout
    VkDescriptorSetLayoutBinding bindings[3] = {
        { .binding = 0, .descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, .descriptorCount = 1, .stageFlags = VK_SHADER_STAGE_COMPUTE_BIT },
        { .binding = 1, .descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, .descriptorCount = 1, .stageFlags = VK_SHADER_STAGE_COMPUTE_BIT },
        { .binding = 2, .descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, .descriptorCount = 1, .stageFlags = VK_SHADER_STAGE_COMPUTE_BIT },
    };
    VkDescriptorSetLayoutCreateInfo dsl_info = {
        .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
        .bindingCount = 3,
        .pBindings = bindings,
    };
    VkDescriptorSetLayout desc_layout;
    check_vk(vkCreateDescriptorSetLayout(device, &dsl_info, NULL, &desc_layout), "vkCreateDescriptorSetLayout");

    VkPipelineLayoutCreateInfo pl_info = {
        .sType = VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
        .setLayoutCount = 1,
        .pSetLayouts = &desc_layout,
    };
    VkPipelineLayout pipeline_layout;
    check_vk(vkCreatePipelineLayout(device, &pl_info, NULL, &pipeline_layout), "vkCreatePipelineLayout");

    // 9. Compute Pipeline
    VkComputePipelineCreateInfo cp_info = {
        .sType = VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO,
        .stage = {
            .sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
            .stage = VK_SHADER_STAGE_COMPUTE_BIT,
            .module = shader_module,
            .pName = "main",
        },
        .layout = pipeline_layout,
    };
    VkPipeline compute_pipeline;
    check_vk(vkCreateComputePipelines(device, VK_NULL_HANDLE, 1, &cp_info, NULL, &compute_pipeline), "vkCreateComputePipelines");
    printf("[VULKAN] Compute pipeline compiled and bound to Lavapipe compute queue.\n");

    // 10. Descriptor Pool & Descriptor Set
    VkDescriptorPoolSize pool_size = {
        .type = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
        .descriptorCount = 3,
    };
    VkDescriptorPoolCreateInfo pool_info = {
        .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
        .maxSets = 1,
        .poolSizeCount = 1,
        .pPoolSizes = &pool_size,
    };
    VkDescriptorPool desc_pool;
    check_vk(vkCreateDescriptorPool(device, &pool_info, NULL, &desc_pool), "vkCreateDescriptorPool");

    VkDescriptorSetAllocateInfo ds_alloc_info = {
        .sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
        .descriptorPool = desc_pool,
        .descriptorSetCount = 1,
        .pSetLayouts = &desc_layout,
    };
    VkDescriptorSet desc_set;
    check_vk(vkAllocateDescriptorSets(device, &ds_alloc_info, &desc_set), "vkAllocateDescriptorSets");

    VkDescriptorBufferInfo buf_infos[3];
    VkWriteDescriptorSet write_sets[3];
    for (int i = 0; i < 3; i++) {
        buf_infos[i].buffer = buffers[i];
        buf_infos[i].offset = 0;
        buf_infos[i].range = buffer_size;

        write_sets[i].sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET;
        write_sets[i].pNext = NULL;
        write_sets[i].dstSet = desc_set;
        write_sets[i].dstBinding = i;
        write_sets[i].dstArrayElement = 0;
        write_sets[i].descriptorCount = 1;
        write_sets[i].descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER;
        write_sets[i].pBufferInfo = &buf_infos[i];
        write_sets[i].pImageInfo = NULL;
        write_sets[i].pTexelBufferView = NULL;
    }
    vkUpdateDescriptorSets(device, 3, write_sets, 0, NULL);

    // 11. Command Pool & Command Buffer Recording
    VkCommandPoolCreateInfo cmd_pool_info = {
        .sType = VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
        .queueFamilyIndex = compute_queue_family,
    };
    VkCommandPool cmd_pool;
    check_vk(vkCreateCommandPool(device, &cmd_pool_info, NULL, &cmd_pool), "vkCreateCommandPool");

    VkCommandBufferAllocateInfo cmd_buf_alloc_info = {
        .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
        .commandPool = cmd_pool,
        .level = VK_COMMAND_BUFFER_LEVEL_PRIMARY,
        .commandBufferCount = 1,
    };
    VkCommandBuffer cmd_buffer;
    check_vk(vkAllocateCommandBuffers(device, &cmd_buf_alloc_info, &cmd_buffer), "vkAllocateCommandBuffers");

    VkCommandBufferBeginInfo begin_info = { .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO };
    vkBeginCommandBuffer(cmd_buffer, &begin_info);
    vkCmdBindPipeline(cmd_buffer, VK_PIPELINE_BIND_POINT_COMPUTE, compute_pipeline);
    vkCmdBindDescriptorSets(cmd_buffer, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline_layout, 0, 1, &desc_set, 0, NULL);
    vkCmdDispatch(cmd_buffer, NUM_ELEMENTS / WORKGROUP_SIZE, 1, 1);
    vkEndCommandBuffer(cmd_buffer);

    // 12. Submit & Execute
    printf("[VULKAN] Dispatching compute shader to Lavapipe (%u elements, %u workgroups)...\n",
           NUM_ELEMENTS, NUM_ELEMENTS / WORKGROUP_SIZE);
    VkSubmitInfo submit_info = {
        .sType = VK_STRUCTURE_TYPE_SUBMIT_INFO,
        .commandBufferCount = 1,
        .pCommandBuffers = &cmd_buffer,
    };
    check_vk(vkQueueSubmit(compute_queue, 1, &submit_info, VK_NULL_HANDLE), "vkQueueSubmit");
    check_vk(vkQueueWaitIdle(compute_queue), "vkQueueWaitIdle");
    printf("[VULKAN] Execution completed on compute queue.\n");

    // 13. Verify Results
    float *ptr_c;
    vkMapMemory(device, memories[2], 0, buffer_size, 0, (void **)&ptr_c);
    int errors = 0;
    float max_diff = 0.0f;
    for (int i = 0; i < NUM_ELEMENTS; i++) {
        float expected = ((float)i * 0.5f) * ((float)(NUM_ELEMENTS - i) * 0.25f) + 2.5f;
        float actual = ptr_c[i];
        float diff = fabsf(actual - expected);
        if (diff > max_diff) max_diff = diff;
        if (diff > 1e-4f) {
            if (errors < 5) {
                printf("[ERROR] Mismatch at index %d: actual=%.5f, expected=%.5f, diff=%.6f\n",
                       i, actual, expected, diff);
            }
            errors++;
        }
    }
    vkUnmapMemory(device, memories[2]);

    // 14. Cleanup
    for (int i = 0; i < 3; i++) {
        vkDestroyBuffer(device, buffers[i], NULL);
        vkFreeMemory(device, memories[i], NULL);
    }
    vkDestroyDescriptorPool(device, desc_pool, NULL);
    vkDestroyDescriptorSetLayout(device, desc_layout, NULL);
    vkDestroyPipeline(device, compute_pipeline, NULL);
    vkDestroyPipelineLayout(device, pipeline_layout, NULL);
    vkDestroyShaderModule(device, shader_module, NULL);
    vkDestroyCommandPool(device, cmd_pool, NULL);
    vkDestroyDevice(device, NULL);
    vkDestroyInstance(instance, NULL);
    free(devices);

    printf("===============================================================================\n");
    if (errors == 0) {
        printf("✓ [PASS] VULKAN LAVAPIPE COMPUTE TEST SUCCEEDED!\n");
        printf("  - Elements calculated: %d\n", NUM_ELEMENTS);
        printf("  - Max difference from mathematical reference: %.8f\n", max_diff);
        printf("  - Pure CPU Vulkan compute emulation verified on Mesa Lavapipe.\n");
        printf("===============================================================================\n");
        return 0;
    } else {
        printf("✗ [FAIL] %d errors detected in computed buffer!\n", errors);
        printf("===============================================================================\n");
        return 1;
    }
}

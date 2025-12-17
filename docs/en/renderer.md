# platform::renderer

### Startup Flow
1. Process start
   - The renderer is started by command line or a process manager (`GpuRenderer::new()`).

2. Load initial configuration
   - Load settings (the `RendererConfig` struct). Required fields include: width, height, backend, enable_vsync, memory_limits.
   - Configuration validation: startup fails on invalid values (fatal error). If an error is returned to the main process, notify it during the startup handshake.

3. Initialize logging and metrics
   - Initialize structured logging (e.g., JSON).
   - Register metrics collection pipeline (frame time, memory, GPU utilization).

4. Initialize GPU backend
   - Initialize the GPU context for the selected backend (physical device selection, swapchain/surface creation, command pool creation, etc.).
   - On failure: abort startup and return details to main (`RenderError::GpuInit`).

5. Create IPC endpoint (listen)
   - Create an OS-native named pipe (Windows) or a Unix domain socket / TCP listener and wait for connections.
   - Start an async listener and prepare to accept connections from the main process.

Output: GPU context established and IPC listening.

### Handshake & Authentication Flow

1. Main process establishes connection
   - A socket connection is opened by the main process, or a preopened handle provided by the parent is used to establish the communication channel.

2. Exchange versions and capabilities
   - Each side exchanges messages announcing protocol version, supported features (e.g., extended shaders, capture_frame), and message compression methods.
   - Example messages: `HandshakeRequest { version: 1, features: [...] }` / `HandshakeResponse { accepted: true, renderer_version: "x.y" }`.

3. Authentication (optional but recommended)
   - Validate tokens or inter-process signatures provided via startup arguments to authorize communication.
   - On failure: drop the connection and log the event.

4. Sync initial state
   - Receive initial rendering state from main (initial DrawList/scene tree) and a list of required resources (textures, fonts).

On success: the renderer is ready to enter the main command receive loop.

### Initial Resource Load Flow

1. Receive resource requests
   - Receive a list of `ResourceRequest` items from main and assign priorities (whether the resource is critical for display).

2. Delegate to asynchronous loading workers
   - File I/O and decoding (PNG/JPEG/font decoding) happen on worker threads.
   - When loading completes, notify main with `ResourceLoaded(id)` or upload immediately internally.

3. GPU upload
   - When data is ready for GPU transfer, push it to a deferred upload queue.
   - Uploads are executed at GPU submission points to avoid race conditions.

4. Fallbacks
   - On load failure, prepare a placeholder (solid-color texture, etc.) and notify main with a `ResourceError`.


### Main command receive loop (message processing loop)

1. Receiver thread / async loop
   - IPC receives messages asynchronously and pushes incoming `RenderCommand` values into the receive queue.
   - Heavy work (decoding, font generation) is handed off to worker threads.

2. Command prioritization
   - Time-critical commands (Resize, Draw) are given higher priority.
   - Batching: multiple Draw commands within the same frame are coalesced for efficiency.

3. Apply commands
   - Dequeue commands and apply them to internal state (scene graph, layer state).
   - Resolve resource references as needed (if missing, request `ResourceRequest` from main or use a placeholder).

4. Frame generation trigger
   - If received commands require frame generation (explicit `Draw` or a VSync trigger), call `render_frame`.
   - If periodic updates exist (VSync/Timer), those triggers can also invoke rendering.

### Frame generation (render_frame) flow — per-stage details

Note: This flow describes the synchronous path for a single frame up to GPU submission.

1. Command flattening
   - Determine the DrawList to render for this frame from the current state.
   - Use per-layer dirty flags to avoid unnecessary redraws.

2. World-to-screen transform (coordinate/clip)
   - Transform each DrawCommand's coordinate space to screen space.
   - Compute clip regions and perform simple occlusion to reduce rendering load.

3. Prepass (CPU-side preparation)
   - Build vertex/index buffers and assemble uniform data.
   - Register textures that are not yet uploaded into the upload queue.

4. Batching and sorting
   - Sort and batch draw calls by shader / blend mode / texture to minimize state changes.

5. GPU command creation
   - Record GPU command buffers and write the required pipeline bindings and draw commands.

6. GPU submit
   - Submit the command buffers to the GPU and optionally insert fences.
   - In asynchronous paths the CPU returns to other work immediately; be careful with resource lifetime management.

7. Presentation
   - Present to the swapchain / surface (respect vsync when enabled).
   - On success, optionally emit a `RenderEvent::FramePresented` to main.

8. Post-processing (if needed)
   - If a frame capture was requested, synchronously read pixels and return a `CapturedFrame`.
   - Update GPU memory usage statistics and evaluate garbage collection triggers.

Success criteria: Frame generation completed (submitted to GPU) or an explicit error returned to main.

### Resource management flow (Resource lifecycle)

1. Resource identification
   - Each resource is managed by a unique ID (e.g., `Uuid`).
   - Use reference counting plus generation numbers to invalidate stale handles.

2. Lifecycle: Request → Load → Upload → Use → Evict
   - Request: Receive `LoadResource(id, kind, data/url)` from main.
   - Load: Decode and optimize (mipmap generation, etc.) on worker threads.
   - Upload: Transfer to the GPU via the upload queue.
   - Use: Referenced by DrawCommands.
   - Evict: Release from GPU/CPU due to LRU, budget overrun, or explicit `UnloadResource`.

3. Allocation policy
   - Configure GPU memory budget via config. Evict low-priority resources when over budget.
   - Critical resources (main UI textures) can be pinned manually.

4. Errors and fallbacks
   - Upload or decode failures produce `RenderEvent::Error(Resource(...))` and fall back to placeholders.

Generated by chatGPT-5
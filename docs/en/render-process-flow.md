# Prerequisites
## Creating the application's main event loop
`EventLoop` is the mechanism that receives events from the OS and delivers them to the application.
```rust
let event_loop = EventLoop::<State>::with_user_event().build()?;
let mut app = App::new();

event_loop.run_app(&mut app)?;
```
### What happens inside `run_app()`
1. **Create the `EventLoop`**
   * Prepare communication with the OS.
   * The loop runs until the application is closed (an infinite loop from the app's point of view).

2. **Call `run_app`**
   * The event loop starts.
   * It processes OS events in order and forwards them to the `App`.
   * The `App` receives events through functions like `update()` and `render()`.

3. **Run until the app exits**
   * Closing the window triggers events such as `Event::LoopDestroyed` and the loop ends.


### Roles summary
| Concept       | Role                                                                |
|---------------|---------------------------------------------------------------------|
| **EventLoop** | A bridge between the OS and the app; the entrypoint for all events. |
| **App**       | Your application's logic; it reacts to received events.             |
| **run_app()** | Starts the loop and runs the application.                           |


# What we implement in Orinium
## `GpuRenderer`
* ./src/engine/renderer
Lifecycle
```rust
App::resumed()
 └── State::new()
      └── GpuRenderer::new()   ← GPU initialization
App::set_draw_commands()
 └── GpuRenderer::update_draw_commands()
EventLoop: RedrawRequested
 └── State::render()
      └── GpuRenderer::render() ← per-frame rendering
EventLoop: Resized
 └── State::resize()
      └── GpuRenderer::resize()
App shutdown
 └── GpuRenderer is dropped and GPU resources are freed

```

| Phase    | Function                 | Primary responsibilities          | When it's called                       |
|----------|--------------------------|-----------------------------------|----------------------------------------|
| Init     | `new()`                  | Create GPU instance and pipelines | On app startup (`resumed()`)           |
| Update   | `update_draw_commands()` | Update vertex and text buffers    | When shapes or text change             |
| Render   | `render()`               | Render a single frame             | Each frame or after `request_redraw()` |
| Resize   | `resize()`               | Update buffers and font regions   | On window size changes                 |
| Shutdown | `Drop`                   | Free GPU memory and resources     | On app exit                            |

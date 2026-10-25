// Full-fake capture mechanism informed by hyprexpo (MIT); implementation original.
#include "overview.hpp"
#include <chrono>
#include <hyprland/src/event/EventBus.hpp>
#include <hyprland/src/output/Monitor.hpp>
#include <hyprland/src/render/Renderer.hpp>
#include <hyprland/src/render/gl/GLFramebuffer.hpp>
#include <hyprland/src/state/MonitorState.hpp>
#include <hyprland/src/state/workspace/State.hpp>
#include <hyprland/src/workspace/HLWorkspace.hpp>
#include <hyprutils/utils/ScopeGuard.hpp>

namespace OverviewProbe {
// Expose protected base member pointers without casting the renderer to a
// fictitious derived object or changing access with preprocessor macros.
struct RendererAccess : Render::IHyprRenderer {
    static auto workspaceDraw() { return &RendererAccess::renderWorkspace; }
    static auto windowsDraw() { return &RendererAccess::renderWorkspaceWindows; }
    static auto mode() { return &RendererAccess::m_renderMode; }
};
static rust::Vec<hypr2d::ProbeCell> plan;
static std::vector<SP<Render::IFramebuffer>> images;
static std::vector<double> times;
static CHyprSignalListener stageListener, removeListener;
static uint32_t restored = 0;
static bool deleted = true, capturing = false;
static std::string renderer;
static PHLMONITOR findMonitor(uint32_t id) {
    for (auto &m : State::monitorState()->monitors())
        if (m->m_id == id)
            return m;
    return nullptr;
}
static PHLWORKSPACE findWorkspace(const hypr2d::ProbeCell &cell) {
    for (auto &w : State::Workspace::state()->workspaces())
        if ((!cell.special.empty() && w->addressableName() == std::string(cell.special)) ||
            (cell.special.empty() && w->numberedID() && *w->numberedID() == cell.workspace))
            return w.lock();
    return nullptr;
}
struct WorkspaceState {
    PHLWORKSPACE ws;
    float alpha;
    Vector2D offset;
    bool force;
    bool visible;
    float alphaGoal, alphaBegun;
    Vector2D offsetGoal, offsetBegun;
    bool alphaAnimated, offsetAnimated;
};
// Capture runs between frames, synchronously. Only current animation values are
// overridden, never goals, timing, callbacks, registrations, or visibility.
SP<Render::IFramebuffer> capture(const hypr2d::ProbeCell &cell, SP<Render::IFramebuffer> fb, bool finish) {
    auto &r = *g_pHyprRenderer;
    auto m = findMonitor(cell.monitor);
    if (!m)
        throw std::runtime_error("probe output disappeared");
    if (r.m_renderData.currentFB)
        throw std::runtime_error("probe captures must run between frames");
    const auto target = findWorkspace(cell);
    if (target && target->m_monitor != m)
        throw std::runtime_error("probe target belongs to another monitor");
    auto savedData = r.m_renderData;
    const auto savedMode = r.*RendererAccess::mode();

    const bool feedback = r.m_bBlockSurfaceFeedback, snapshot = r.m_bRenderingSnapshot;
    const auto active = m->m_activeWorkspace, special = m->m_activeSpecialWorkspace;
    const auto dim = m->m_specialDim->value(), blur = m->m_specialBlur->value();
    const auto damage = m->m_damage;
    const auto blurShouldRender = m->m_blurFBShouldRender;
    GLint viewport[4], readFB, drawFB;
    glGetIntegerv(GL_VIEWPORT, viewport);
    glGetIntegerv(GL_READ_FRAMEBUFFER_BINDING, &readFB);
    glGetIntegerv(GL_DRAW_FRAMEBUFFER_BINDING, &drawFB);
    const auto usedBuffers = m->m_usedAsyncBuffers;
    std::vector<WorkspaceState> states;
    for (auto &w : State::Workspace::state()->workspaces()) {
        states.push_back({w.lock(), w->m_alpha->value(), w->m_renderOffset->value(), w->m_forceRendering, w->visible(), w->m_alpha->goal(),
                          w->m_alpha->begun(), w->m_renderOffset->goal(), w->m_renderOffset->begun(), w->m_alpha->isBeingAnimated(),
                          w->m_renderOffset->isBeingAnimated()});
    }
    auto savedPass = std::move(r.m_renderPass);
    Hyprutils::Utils::CScopeGuard restore([&] {
        bool animationsIntact = true;
        for (auto &s : states) {
            s.ws->m_alpha->value() = s.alpha;
            s.ws->m_renderOffset->value() = s.offset;
            s.ws->m_forceRendering = s.force;
            animationsIntact &= s.ws->visible() == s.visible && s.ws->m_alpha->goal() == s.alphaGoal &&
                                s.ws->m_alpha->begun() == s.alphaBegun && s.ws->m_renderOffset->goal() == s.offsetGoal &&
                                s.ws->m_renderOffset->begun() == s.offsetBegun && s.ws->m_alpha->isBeingAnimated() == s.alphaAnimated &&
                                s.ws->m_renderOffset->isBeingAnimated() == s.offsetAnimated;
        }
        m->m_activeWorkspace = active;
        m->m_activeSpecialWorkspace = special;
        m->m_specialDim->value() = dim;
        m->m_specialBlur->value() = blur;
        m->m_damage = damage;
        m->m_blurFBShouldRender = blurShouldRender;
        m->m_usedAsyncBuffers = usedBuffers;
        r.m_renderPass.clear();
        r.m_renderPass = std::move(savedPass);
        r.m_renderData = savedData;
        r.*RendererAccess::mode() = savedMode;
        r.m_bBlockSurfaceFeedback = feedback;
        r.m_bRenderingSnapshot = snapshot;
        if (savedData.currentFB)
            savedData.currentFB->bind();
        r.glBackend()->bindFramebuffer(GL_READ_FRAMEBUFFER, readFB);
        r.glBackend()->bindFramebuffer(GL_DRAW_FRAMEBUFFER, drawFB);
        r.glBackend()->setViewport(viewport[0], viewport[1], viewport[2], viewport[3]);
        if (animationsIntact)
            ++restored;
    });
    for (auto &s : states) {
        s.ws->m_alpha->value() = 0.f;
        s.ws->m_forceRendering = false;
    }
    m->m_activeWorkspace = target;
    m->m_activeSpecialWorkspace.reset();
    m->m_specialDim->value() = 0.f;
    m->m_specialBlur->value() = 0.f;
    if (target) {
        target->m_alpha->value() = 1.f;
        target->m_renderOffset->value() = Vector2D{};
        target->m_forceRendering = true;
    }
    r.m_bBlockSurfaceFeedback = true;
    r.m_bRenderingSnapshot = true;
    if (!fb)
        fb = r.createFB("hypr2d-thumbnail");
    if (!fb->alloc(cell.capture_width, cell.capture_height, DRM_FORMAT_ARGB8888))
        throw std::runtime_error("probe framebuffer allocation failed");
    fb->getTexture()->bind();
    GLint textureWidth = 0, textureHeight = 0, internalFormat = 0;
    glGetTexLevelParameteriv(GL_TEXTURE_2D, 0, GL_TEXTURE_WIDTH, &textureWidth);
    glGetTexLevelParameteriv(GL_TEXTURE_2D, 0, GL_TEXTURE_HEIGHT, &textureHeight);
    glGetTexLevelParameteriv(GL_TEXTURE_2D, 0, GL_TEXTURE_INTERNAL_FORMAT, &internalFormat);
    fb->getTexture()->unbind();
    if (textureWidth != int(cell.capture_width) || textureHeight != int(cell.capture_height) || internalFormat != GL_RGBA8 ||
        fb->getStencilTex())
        throw std::runtime_error("probe texture storage differs from RGBA8 budget");
    fb->setImageDescription(m->workBufferImageDescription());
    // Surface rendering intersects damage in source coordinates before applying
    // render modifiers. Full source damage is essential at thumbnail sizes.
    CRegion damageRegion{0, 0, m->m_transformedSize.x, m->m_transformedSize.y};
    if (!r.beginFullFakeRender(m, damageRegion, fb))
        throw std::runtime_error("probe full-fake begin failed");
    if (r.m_renderData.currentFB != fb || r.m_renderData.currentFB->m_size != fb->m_size)
        throw std::runtime_error("capture is not rendering directly into thumbnail");
    if (!cell.special.empty()) {
        r.m_renderPass.add(makeUnique<CClearPassElement>(CClearPassElement::SClearData{CHyprColor(0.02, 0.02, 0.02, 1.0)}));
        Render::SRenderModifData mod;
        mod.modifs.emplace_back(Render::SRenderModifData::RMOD_TYPE_SCALE, float(cell.capture_width / m->m_transformedSize.x));
        r.m_renderPass.add(makeUnique<CRendererHintsPassElement>(CRendererHintsPassElement::SData{mod}));
        (r.*RendererAccess::windowsDraw())(m, target, Time::steadyNow());
        r.m_renderPass.add(makeUnique<CRendererHintsPassElement>(CRendererHintsPassElement::SData{Render::SRenderModifData{}}));
    } else
        (r.*RendererAccess::workspaceDraw())(
            m, target, Time::steadyNow(),
            CBox{0, 0, double(cell.capture_width), cell.capture_width * m->m_transformedSize.y / m->m_transformedSize.x});
    r.endRender();
    // Simple FULL_FAKE renders directly into the destination: no full-output
    // work buffer or export resample. At settled size composition samples level
    // zero once; avoid trilinear mip blending softening near-1:1 thumbnails.
    fb->getTexture()->minFilter = GL_LINEAR;
    fb->getTexture()->magFilter = GL_NEAREST;
    if (finish)
        glFinish(); // Test-only completion measurement.
    return fb;
}
class ProbePass final : public IPassElement {
  public:
    explicit ProbePass(PHLMONITOR m) : monitor(m) {}
    std::vector<UP<IPassElement>> draw() override {
        std::vector<UP<IPassElement>> result;
        const double width = monitor->m_transformedSize.x;
        result.push_back(makeUnique<CRectPassElement>(
            CRectPassElement::SRectData{.box = {0, 0, width, monitor->m_transformedSize.y}, .color = CHyprColor(0.035, 0.04, 0.05, 1.0)}));
        for (size_t i = 0; i < images.size(); ++i) {
            const auto &c = plan[i];
            result.push_back(makeUnique<CTexPassElement>(CTexPassElement::SRenderData{
                .tex = images[i]->getTexture(), .box = {c.x * width, c.y * width, c.width * width, c.height * width}}));
        }
        return result;
    }
    bool needsLiveBlur() override { return false; }
    bool needsPrecomputeBlur() override { return false; }
    const char *passName() override { return "hypr2d-probe"; }
    ePassElementType type() override { return EK_CUSTOM; }
    bool disableSimplification() override { return true; }
    bool requiresFullDamage() override { return true; }

  private:
    PHLMONITOR monitor;
};
void close() {
    auto &r = *g_pHyprRenderer;
    r.glBackend()->makeEGLCurrent();
    r.m_renderPass.removeAllOfType("hypr2d-probe");
    std::vector<GLuint> textures, buffers;
    for (auto &fb : images) {
        textures.push_back(fb->getTexture()->m_texID);
        buffers.push_back(dynamicPointerCast<Render::GL::CGLFramebuffer>(fb)->getFBID());
    }
    images.clear();
    glFinish();
    deleted = true;
    for (auto id : textures)
        deleted &= !glIsTexture(id);
    for (auto id : buffers)
        deleted &= !glIsFramebuffer(id);
    for (auto &m : State::monitorState()->monitors()) {
        r.damageMonitor(m);
        m->scheduleFrame(Aquamarine::IOutput::AQ_SCHEDULE_DAMAGE);
    }
}
void init() {
    stageListener = Event::bus()->m_events.render.stage.listen([](eRenderStage stage) {
        if (!capturing && !images.empty() && stage == RENDER_LAST_MOMENT)
            g_pHyprRenderer->m_renderPass.add(makeUnique<ProbePass>(g_pHyprRenderer->m_renderData.pMonitor.lock()));
    });
    removeListener = Event::bus()->m_events.monitor.preRemoved.listen([](PHLMONITOR) { close(); });
}
void shutdown() {
    stageListener.reset();
    removeListener.reset();
    close();
    plan.clear();
    times.clear();
}
std::string command(hypr2d::Core &core, const std::vector<hypr2d::Monitor> &monitors, const std::string &input) {
    if (input == "probe close")
        close();
    else if (input == "probe open" || input == "probe full") {
        close();
        if (g_pHyprRenderer->type() != Render::IHyprRenderer::RT_GL)
            return "error: GL prototype only";
        std::vector<hypr2d::ProbeSpecial> specials;
        for (auto &w : State::Workspace::state()->workspaces())
            if (w->type() == Workspace::eWorkspaceType::SPECIAL && w->m_monitor)
                specials.push_back({static_cast<uint32_t>(w->m_monitor->m_id), w->addressableName()});
        plan = core.probe_plan({monitors.data(), monitors.size()}, {specials.data(), specials.size()}, input == "probe full");
        times.clear();
        restored = 0;
        capturing = true;
        Hyprutils::Utils::CScopeGuard finish([] { capturing = false; });
        g_pHyprRenderer->glBackend()->makeEGLCurrent();
        renderer = reinterpret_cast<const char *>(glGetString(GL_RENDERER));
        for (const auto &c : plan) {
            const auto begin = std::chrono::steady_clock::now();
            images.push_back(capture(c, {}, true));
            times.push_back(std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - begin).count());
        }
        deleted = false;
        for (auto &m : State::monitorState()->monitors())
            g_pHyprRenderer->damageMonitor(m);
    } else if (input != "probe stats")
        return "error: expected probe open/full/close/stats";
    return std::string(
        hypr2d::probe_report({plan.data(), plan.size()}, {times.data(), times.size()}, !images.empty(), deleted, restored, renderer));
}
} // namespace OverviewProbe

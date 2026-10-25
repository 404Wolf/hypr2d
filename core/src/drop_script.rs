//! Public Lua operations only. Never exchange targets between layout spaces.
use crate::{ffi, guarded};
fn quote(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() && (c as u32) <= 127 => out.push_str(&format!("\\{:03}", c as u32)),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}
pub fn script(drop: &ffi::UIDrop, source: &str, x: f64, y: f64) -> String {
    guarded(|| {
        let target = if drop.stack.is_empty() {
            &drop.swap
        } else {
            &drop.stack
        };
        let w = quote(&format!("address:{}", drop.window));
        let t = quote(&format!("address:{target}"));
        let dest = quote(&drop.workspace);
        let mut code=format!("local w=hl.get_window({w}); if not w or not w.mapped then return end; local function dispatch(d) local result=hl.dispatch(d); assert(result.ok,result.error) end; ");
        code.push_str("local saved={}; local focus=hl.get_active_monitor(); for _,m in ipairs(hl.get_monitors()) do saved[#saved+1]={monitor=m,workspace=m.active_workspace,special=m.active_special_workspace} end; local success,failure=pcall(function() ");
        code.push_str(&format!(
            "dispatch(hl.dsp.focus({{monitor={}}})); ",
            drop.monitor
        ));
        if !target.is_empty() {
            code.push_str(&format!(
                "local t=hl.get_window({t}); if not t or not t.mapped then return end; "
            ));
        }
        code.push_str(
            "local function detach(win) if win.group then dispatch(hl.dsp.window.move({window=win,out_of_group=true})) end end; local function move(win,ws) dispatch(hl.dsp.window.move({window=win,workspace=ws,follow=false})); detach(win) end; detach(w); ",
        );
        if !drop.stack.is_empty() {
            code.push_str(&format!("dispatch(hl.dsp.window.move({{window=w,workspace={dest},follow=false}})); if not t.group then dispatch(hl.dsp.group.toggle({{window=t}})) end; assert(t.group,'target group unavailable'); t.group:add(w); dispatch(hl.dsp.group.active({{window=w,index=t.group.current_index}})); "));
        } else if !drop.swap.is_empty() {
            code.push_str(
                "if t.group then dispatch(hl.dsp.window.move({window=t,out_of_group=true})) end; ",
            );
            if source == drop.workspace {
                code.push_str("local wi,wc,ti,tc=w.fullscreen,w.fullscreen_client,t.fullscreen,t.fullscreen_client; dispatch(hl.dsp.window.fullscreen_state({window=w,internal=0,client=0})); dispatch(hl.dsp.window.fullscreen_state({window=t,internal=0,client=0})); dispatch(hl.dsp.window.swap({window=w,target=t})); dispatch(hl.dsp.window.fullscreen_state({window=w,internal=wi,client=wc})); dispatch(hl.dsp.window.fullscreen_state({window=t,internal=ti,client=tc})); ");
            } else {
                code.push_str(&format!("move(w,{dest}); move(t,{}); ", quote(source)));
            }
        } else {
            code.push_str(&format!("move(w,{dest}); "));
            if drop.floating {
                code.push_str(&format!(
                    "dispatch(hl.dsp.window.move({{window=w,x={x},y={y},relative=false}})); "
                ));
            }
        }
        code.push_str(&format!("end); for _,s in ipairs(saved) do local m=s.monitor; if m and m.enabled then if m.active_workspace~=s.workspace and s.workspace then dispatch(hl.dsp.focus({{monitor=m}})); dispatch(hl.dsp.focus({{workspace=s.workspace}})) end; local wanted=s.special; if wanted and wanted.is_empty and wanted.addressable_name=={} then wanted=nil end; if m.active_special_workspace~=wanted then dispatch(hl.dsp.focus({{monitor=m}})); if m.active_special_workspace then dispatch(hl.dsp.workspace.toggle_special(m.active_special_workspace.addressable_name:sub(9))) end; if wanted then dispatch(hl.dsp.workspace.toggle_special(wanted.addressable_name:sub(9))) end end end end; if focus then dispatch(hl.dsp.focus({{monitor=focus}})) end; if not success then error(failure) end; ",quote(source)));
        code
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn special_swap_uses_two_silent_dispatches_and_quotes_names() {
        let d = ffi::UIDrop {
            window: "0xa".into(),
            swap: "0xb".into(),
            workspace: "special:a\"\\\n".into(),
            ..Default::default()
        };
        let s = script(&d, "21", 0., 0.);
        assert!(s.contains("move(w,") && s.contains("move(t,\"21\")"));
        assert!(!s.contains("window.swap"));
        assert!(s.contains("special:a\\\"\\\\\\010"));
        assert_eq!(quote("λ"), "\"λ\"");
    }
}

//! The set of Arkime components the admin chose to configure. Unlike the old
//! bash `Configure` (which had mutually-exclusive `--wise`/`--parliament`/
//! `--cont3xt`/default modes), the new tool lets any combination be toggled on,
//! and the downstream prompts/artifacts are the *union* of what the enabled
//! components require.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    Capture,
    Viewer,
    Wise,
    Parliament,
    Cont3xt,
}

impl Component {
    /// All components, in display order.
    pub const ALL: [Component; 5] = [
        Component::Capture,
        Component::Viewer,
        Component::Wise,
        Component::Parliament,
        Component::Cont3xt,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Component::Capture => "capture",
            Component::Viewer => "viewer",
            Component::Wise => "wise",
            Component::Parliament => "parliament",
            Component::Cont3xt => "cont3xt",
        }
    }

    /// systemd/rc.d service unit name (matches the units shipped by Arkime).
    pub fn service_name(self) -> &'static str {
        match self {
            Component::Capture => "arkimecapture",
            Component::Viewer => "arkimeviewer",
            Component::Wise => "arkimewise",
            Component::Parliament => "arkimeparliament",
            Component::Cont3xt => "arkimecont3xt",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Components {
    pub capture: bool,
    pub viewer: bool,
    pub wise: bool,
    pub parliament: bool,
    pub cont3xt: bool,
}

impl Components {
    pub fn contains(&self, c: Component) -> bool {
        match c {
            Component::Capture => self.capture,
            Component::Viewer => self.viewer,
            Component::Wise => self.wise,
            Component::Parliament => self.parliament,
            Component::Cont3xt => self.cont3xt,
        }
    }

    pub fn toggle(&mut self, c: Component) {
        let slot = match c {
            Component::Capture => &mut self.capture,
            Component::Viewer => &mut self.viewer,
            Component::Wise => &mut self.wise,
            Component::Parliament => &mut self.parliament,
            Component::Cont3xt => &mut self.cont3xt,
        };
        *slot = !*slot;
    }

    pub fn any(&self) -> bool {
        self.capture || self.viewer || self.wise || self.parliament || self.cont3xt
    }

    pub fn enabled(&self) -> impl Iterator<Item = Component> + '_ {
        Component::ALL.into_iter().filter(|&c| self.contains(c))
    }

    /// Only capture actually sniffs an interface.
    pub fn needs_interfaces(&self) -> bool {
        self.capture
    }

    /// Anything that talks to an OpenSearch/Elasticsearch backend.
    pub fn needs_elasticsearch(&self) -> bool {
        self.capture || self.viewer || self.cont3xt
    }

    /// Components that read the shared S2S/encryption secret.
    pub fn needs_s2s_password(&self) -> bool {
        self.capture || self.viewer || self.cont3xt
    }
}

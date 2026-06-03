use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Route {
    #[default]
    Home,
    Session {
        session_id: String,
    },
    Settings,
    Help,
}

impl std::fmt::Display for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Route::Home => write!(f, "Home"),
            Route::Session { session_id } => write!(f, "Session: {}", session_id),
            Route::Settings => write!(f, "Settings"),
            Route::Help => write!(f, "Help"),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Router {
    history: Vec<Route>,
    current: Route,
}

impl Router {
    pub fn new() -> Self {
        Self {
            history: vec![Route::Home],
            current: Route::Home,
        }
    }

    pub fn current(&self) -> &Route {
        &self.current
    }

    pub fn navigate(&mut self, route: Route) {
        if self.current != route {
            self.history.push(self.current.clone());
            self.current = route;
        }
    }

    pub fn go_back(&mut self) -> bool {
        if self.history.len() > 1 {
            if let Some(prev) = self.history.pop() {
                self.current = prev;
                return true;
            }
        }
        false
    }

    pub fn is_home(&self) -> bool {
        matches!(self.current, Route::Home)
    }

    pub fn is_session(&self) -> bool {
        matches!(self.current, Route::Session { .. })
    }

    pub fn session_id(&self) -> Option<&str> {
        match &self.current {
            Route::Session { session_id } => Some(session_id),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Route, Router};

    #[test]
    fn go_back_returns_previous_route() {
        let mut router = Router::new();
        let parent = Route::Session {
            session_id: "parent".to_string(),
        };
        let child = Route::Session {
            session_id: "child".to_string(),
        };

        router.navigate(parent.clone());
        router.navigate(child);

        assert!(router.go_back());
        assert_eq!(router.current(), &parent);

        assert!(router.go_back());
        assert_eq!(router.current(), &Route::Home);

        assert!(!router.go_back());
    }
}

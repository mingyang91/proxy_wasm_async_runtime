use std::ops::Deref;

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::route::{
    radix_tree::{Matches, RadixTree},
    trie::Trie,
    RouteError,
};

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VirtualHost<T> {
    pub host: String,
    pub routes: Vec<Route<T>>,
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Route<T> {
    pub path: String,
    #[serde(flatten)]
    pub config: T,
    pub children: Option<Vec<Route<T>>>,
}

impl<T> TryFrom<Vec<VirtualHost<T>>> for Router<T> {
    type Error = RouteError;

    fn try_from(value: Vec<VirtualHost<T>>) -> Result<Self, Self::Error> {
        let mut trie = Trie::default();
        for virtual_host in value.into_iter() {
            let mut radix = RadixTree::default();
            for route in virtual_host.routes {
                radix_add_all(&mut radix, &route.path, route.config, route.children)?;
            }
            trie.add(&virtual_host.host, radix)?;
        }
        Ok(Router(trie))
    }
}

fn radix_add_all<T>(
    radix: &mut RadixTree<T>,
    path: &str,
    config: T,
    children: Option<Vec<Route<T>>>,
) -> Result<(), RouteError> {
    radix.add(path, config)?;
    let Some(children) = children else {
        return Ok(());
    };

    for child in children {
        let path = normalize_path(&format!("{}/{}", path, child.path));
        radix_add_all(radix, &path, child.config, child.children)?;
    }
    Ok(())
}

fn normalize_path(path: &str) -> String {
    let re = Regex::new("//+").unwrap();
    let mut path = re.replace_all(path, "/").to_string();
    if !path.starts_with('/') {
        path.insert(0, '/');
    }
    path
}

pub struct Router<T>(Trie<RadixTree<T>>);

pub struct Found<'a, T>(Matches<'a, T>);

impl<'a, T> Found<'a, T> {
    pub fn pattern(&self) -> &str {
        &self.0.data.pattern
    }
}

impl<T> Deref for Found<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0.data.data
    }
}

impl<T> Router<T> {
    pub fn matches(&self, domain: &str, path: &str) -> Option<Found<T>> {
        let route = self.0.matches(domain)?;
        route.matches(path).map(|matches| Found(matches))
    }
}

#[cfg(test)]
mod test {
    use crate::cidr::CIDR;

    use super::*;

    #[test]
    fn test_config() {
        let config_str = r#"
  - host: "example.com"
    routes:
      - path: "/"
        rate_limit:
          unit: minute
          requests_per_unit: 100
      - path: "/api"
        rate_limit:
          unit: minute
          requests_per_unit: 50
        children:
          - path: "/users"
            rate_limit:
                unit: minute
                requests_per_unit: 100
          - path: "/posts/*"
            rate_limit:
                unit: minute
                requests_per_unit: 100
  - host: "another-example.com"
    routes:
      - path: "/"
        rate_limit:
          unit: minute
          requests_per_unit: 200
      - path: "/about"
        rate_limit:
          unit: minute
          requests_per_unit: 100
        "#;

        let config: Vec<VirtualHost<serde_yaml::Value>> =
            serde_yaml::from_str(config_str).expect("failed to parse config");
        let route: Router<serde_yaml::Value> = config.try_into().expect("failed to convert config");

        let found = route
            .matches("example.com", "/api/posts/114514")
            .expect("route not found");
        println!("{:?}", found.clone());
    }

    #[test]
    fn cidr_contains() {
        let cidr: CIDR = "192.168.0.0/24".parse().unwrap();
        assert!(cidr.contains("192.168.0.250".parse().unwrap()));
        assert!(!cidr.contains("192.168.10.250".parse().unwrap()));

        let cidr: CIDR = "2001:db8::/32".parse().unwrap();
        assert!(cidr.contains("2001:db8::1".parse().unwrap()));
        assert!(cidr.contains("2001:db8::ffff".parse().unwrap()));
    }

    #[test]
    fn print_v6_cidr() {
        let cidr: CIDR = "2001:db8::/32".parse().unwrap();
        assert_eq!(format!("{}", cidr), "2001:db8::/32");
        let cidr: CIDR = "1111::abcd:0:0:1234:abcd/64".parse().unwrap();
        assert_eq!(format!("{}", cidr), "1111::abcd:0:0:1234:abcd/64");
        let cidr: CIDR = "::/0".parse().unwrap();
        assert_eq!(format!("{}", cidr), "::/0");
        let cidr: CIDR = "1050::5:600:300c:326b/128".parse().unwrap();
        assert_eq!(format!("{}", cidr), "1050::5:600:300c:326b/128");
        let cidr: CIDR = "1050::5:600:300c:326b/128".parse().unwrap();
        assert_eq!(format!("{}", cidr), "1050::5:600:300c:326b/128");
    }
}

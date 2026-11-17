#![allow(dead_code)]

use meridian_ir::{parse, Document};

pub const VALID_INFRASTRUCTURE: &str = r#"apiVersion: meridian/v1
kind: Infrastructure
metadata:
  name: web-vm
spec:
  target: terraform
  resources:
    - id: web_vm
      type: compute_instance
      params:
        image: ubuntu-22.04
        size: small
  outputs:
    - name: web_vm_ip
      value: "${web_vm.public_ip}"
"#;

pub const VALID_RESOURCE_SET: &str = r#"apiVersion: meridian/v1
kind: ResourceSet
metadata:
  name: web-server-baseline
spec:
  target: ansible
  targetHost:
    from: infrastructure
    ref: web-vm
    output: web_vm_ip
  resources:
    - id: nginx_pkg
      type: package
      state: present
      params:
        name: nginx
    - id: nginx_conf
      type: file
      params:
        path: /etc/nginx/nginx.conf
      dependsOn:
        - resource: nginx_pkg
          ifMissing: error
      notifies:
        - resource: nginx_svc
          on: changed
          action: restart
    - id: nginx_svc
      type: service
      state: running
      dependsOn:
        - resource: nginx_pkg
"#;

pub fn bundle_text() -> String {
    format!("{VALID_INFRASTRUCTURE}---\n{VALID_RESOURCE_SET}")
}

pub fn parse_one(doc: &str) -> Document {
    let mut docs = parse("doc.yaml", doc.as_bytes()).unwrap_or_else(|e| panic!("parse: {e}"));
    assert_eq!(docs.len(), 1, "want one document");
    docs.remove(0)
}

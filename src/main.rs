extern crate chrono;

use actix_web::{error, get, post, web, App, HttpResponse, HttpServer, Responder};
use serde::{Deserialize, Serialize};
use tokio_pg_mapper_derive::PostgresMapper;
use deadpool_postgres::{Manager, ManagerConfig, Client, Pool, RecyclingMethod};
use tokio_postgres::{NoTls};
use chrono::Local;
use std::{env, net::{IpAddr, Ipv4Addr}};
use std::{thread, time};

#[derive(PostgresMapper, Deserialize, Serialize)]
#[pg_mapper(table = "transacao")]
pub struct Transacao {
	pub cliente_id: u32,
	pub valor: u32,
	pub tipo: String,
	pub descricao: String,
	pub realizada_em: String,
	pub limite_atual: u32,
	pub saldo_atual: i32,
}

#[derive(PostgresMapper, Deserialize, Serialize)]
#[pg_mapper(table = "cliente")]
pub struct Cliente {
	pub nome: String,
	pub limite: u32,
	pub saldo: i32,
}

#[derive(Deserialize)]
struct TransacaoRequest {
	valor: u32,
	tipo: String,
	descricao: String
}

#[derive(Serialize)]
struct TransacaoResponse {
	saldo: i32,
	limite: u32,
}

#[derive(Serialize)]
struct TransacaoExtratoResponse {
	valor: i32,
	tipo: String,
	descricao: String,
	realizada_em: String,
}

#[derive(Serialize)]
struct SaldoExtratoResponse {
	total: i32,
	limite: i32,
	data_extrato: String,
}

#[derive(Serialize)]
struct ExtratoResponse {
	saldo: SaldoExtratoResponse,
	ultimas_transacoes: Vec<TransacaoExtratoResponse>,
}

fn is_valid_cliente_id(cliente_id: u32) -> bool {
	return cliente_id > 0 && cliente_id < 6;
}

fn get_query_transacao(tipo: String) -> String {
	let query: String;
	let select: String;

	if tipo == "c" {
		query = "UPDATE cliente SET saldo = saldo + $1 WHERE id = $2 RETURNING saldo, limite".to_string();
		select = "SELECT $3, $4, $5, $6, cliente_atualizado.limite, cliente_atualizado.saldo ".to_string();
	} else {
		query = "UPDATE cliente SET saldo = saldo - $1 WHERE id = $2 AND saldo - $3 >= -ABS(limite) RETURNING saldo, limite".to_string();
		select = "SELECT $4, $5, $6, $7, cliente_atualizado.limite, cliente_atualizado.saldo ".to_string();
	}

	return format!(
		concat!(
			"WITH cliente_atualizado AS ({}) ",
			"INSERT INTO transacao (cliente_id, valor, tipo, descricao, limite_atual, saldo_atual) ",
				"{} ",
				"FROM cliente_atualizado ",
				"RETURNING limite_atual, saldo_atual"
		),
		query, select
	);
}

#[post("/clientes/{cliente_id}/transacoes")]
async fn handle_transacao(path: web::Path<u32>, info: web::Json<TransacaoRequest>, db_pool: web::Data<Pool>) -> impl Responder {
	let cliente_id: u32 = path.into_inner();

	if !is_valid_cliente_id(cliente_id) {
		return HttpResponse::NotFound().finish();
	}

	if info.valor <= 0 {
		return HttpResponse::UnprocessableEntity().finish();
	}

	if info.tipo != "c" && info.tipo != "d" {
		return HttpResponse::UnprocessableEntity().finish();
	}

	if info.descricao == "" {
		return HttpResponse::UnprocessableEntity().finish();
	}

	let descricao_len: usize = info.descricao.len();
	if descricao_len < 1 || descricao_len > 10 {
		return HttpResponse::UnprocessableEntity().finish();
	}

	let db_client: Client = db_pool.get().await.unwrap();
	let query: String = get_query_transacao(info.tipo.clone());
	let stmt = db_client.prepare(&query).await.unwrap();
	let mut saldo_atual: i32 = 0;
	let mut limite_atual: u32 = 0;

	if info.tipo == "c" {
		db_client.query(&stmt, &[
			&info.valor,
			&cliente_id,
			&cliente_id,
			&info.valor,
			&info.tipo,
			&info.descricao,
		])
		.await
		.unwrap()
		.iter()
		.for_each(|row| {
			saldo_atual = row.get::<&str, i32>("saldo_atual");
			limite_atual = row.get::<&str, u32>("limite_atual");
		});
	} else {
		db_client.query(&stmt, &[
			&info.valor,
			&cliente_id,
			&info.valor,
			&cliente_id,
			&info.valor,
			&info.tipo,
			&info.descricao,
		])
		.await
		.unwrap()
		.iter()
		.for_each(|row| {
			saldo_atual = row.get::<&str, i32>("saldo_atual");
			limite_atual = row.get::<&str, u32>("limite_atual");
		});
	}

	let response = TransacaoResponse {
		saldo: saldo_atual,
		limite: limite_atual,
	};

	return HttpResponse::Ok().json(web::Json(response));
}

#[get("/clientes/{cliente_id}/extrato")]
async fn handle_extrato(path: web::Path<u32>, db_pool: web::Data<Pool>) -> impl Responder {
	let cliente_id: u32 = path.into_inner();

	if !is_valid_cliente_id(cliente_id) {
		return HttpResponse::NotFound().finish();
	}

	let db_client: Client = db_pool.get().await.unwrap();
	let query: String = format!(
		concat!(
			"SELECT valor, tipo, descricao, realizada_em, limite_atual, saldo_atual ",
			"FROM transacao ",
			"WHERE cliente_id = $1 ",
			"ORDER BY id DESC ",
			"LIMIT 11 " // Deve pegar uma a mais para ignorar a inicial depois, se necessário"
		)
	);
	let stmt = db_client.prepare(&query).await.unwrap();

	let mut limite_atual: Option<i32> = None;
	let mut saldo_atual: Option<i32> = None;

	let mut ultimas_transacoes = db_client
		.query(&stmt, &[
			&cliente_id,
		])
		.await
		.unwrap()
		.iter()
		.map(|row| {
			if limite_atual == None {
				limite_atual = Some(row.get::<&str, i32>("limite_atual"));
			}

			if saldo_atual == None {
				saldo_atual = Some(row.get::<&str, i32>("saldo_atual"));
			}

			return TransacaoExtratoResponse {
				descricao: row.get::<&str, String>("descricao"),
				tipo: row.get::<&str, String>("tipo"),
				valor: row.get::<&str, i32>("valor"),
				realizada_em: row.get::<&str, String>("realizada_em"),
			};
		})
		.collect::<Vec<TransacaoExtratoResponse>>();

	if ultimas_transacoes.is_empty() {
		return HttpResponse::UnprocessableEntity().finish();
	}

	ultimas_transacoes.pop();
	let date = Local::now();

	let response = ExtratoResponse {
		saldo: SaldoExtratoResponse {
			total: saldo_atual.expect("saldo atual"),
			limite: limite_atual.expect("limite atual"),
			data_extrato: date.to_string(), 
		},
		ultimas_transacoes: ultimas_transacoes
	};

	return HttpResponse::Ok().json(web::Json(response));
}

async fn get_connection() -> Result<Pool, &'static str> {
    let mut pg_config = tokio_postgres::Config::new();
	pg_config.hostaddr(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)));
    pg_config.user(env::var("DB_USER").unwrap().as_str());
	pg_config.password(env::var("DB_PASSWORD").unwrap().as_str());
    pg_config.dbname(env::var("DB_NAME").unwrap().as_str());
	pg_config.port(5432); // obter variável de ambiente
    
	let manager_config = ManagerConfig {
        recycling_method: RecyclingMethod::Fast
    };
    
	let mut retries = 1;

	loop {
		let manager = Manager::from_config(pg_config.clone(), NoTls, manager_config.clone());
		let pool = Pool::builder(manager).max_size(16).build();

		if pool.is_ok() {
			return Ok(pool.unwrap());
		}

		println!("Tentando conectar no banco: {}", retries);

		thread::sleep(time::Duration::from_secs(5));

		if retries > 20 {
			break;
		}

		retries += 1;
	}

	return Err("Erro ao conectar com o banco");
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
	let server_port: u16 = 9999;
	let pool = get_connection().await;

	let server =
		HttpServer::new(move || {
			let json_config = web::JsonConfig::default()
				.limit(256)
				.error_handler(|err, _req| {
					return error::InternalError::from_response(err, HttpResponse::UnprocessableEntity().finish()).into()
				});

			let config = web::Data::new(pool.clone());

			App::new()
				.app_data(config)
				.app_data(json_config)
				.service(handle_transacao)
				.service(handle_extrato)
		})
		.bind(("localhost", server_port))?
		.run();

	println!("Servidor rodando na porta: {}", server_port);

	return server.await;
}
